//! Author: Will Hopkins <willothyh@gmail.com>
//! config.rs: Configuration file handling for confinuum

use std::{
    collections::{HashMap, HashSet},
    env::var,
    path::PathBuf,
};

use anyhow::{anyhow, Context, Result};
use common_path::common_path_all;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct Confinuum {
    pub git_protocol: GitProtocol,
    /// Where to look for the user's name and email to be used in git commits
    /// If this is set to github, the user's name and email will be fetched from their github account
    /// If this is set to config, the user's name and email will be fetched from the config file
    pub signature_source: SignatureSource,
}

#[derive(Debug, Deserialize, Serialize)]
pub enum SignatureSource {
    #[serde(rename = "github")]
    Github,
    #[serde(rename = "gitconfig")]
    GitConfig,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ConfigEntry {
    #[serde(skip)]
    pub name: String,
    /// The directory where the files will be deployed
    /// Example: ~/.config/nvim - files from ~/.config/confinuum/nvim will be symlinked to
    /// ~/.config/nvim/<file>
    /// This must be an absolute path
    /// Optional only for uninitialized config, it will always be set when adding files
    pub target_dir: Option<PathBuf>,
    pub files: HashSet<PathBuf>,
}

#[derive(Debug, Deserialize, Serialize)]
pub enum GitProtocol {
    #[serde(rename = "ssh")]
    Ssh,
    #[serde(rename = "https")]
    Https,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ConfinuumConfig {
    pub confinuum: Confinuum,
    #[serde(flatten)]
    pub entries: HashMap<String, ConfigEntry>,
}

impl ConfinuumConfig {
    pub fn init(git_protocol: GitProtocol, signature_source: SignatureSource) -> Self {
        Self {
            confinuum: Confinuum {
                git_protocol,
                signature_source,
            },
            entries: HashMap::new(),
        }
    }

    pub fn add_files_recursive(
        entry: &mut ConfigEntry,
        files: Vec<PathBuf>,
        mut base: Option<PathBuf>,
        result_files: &mut Option<&mut HashSet<PathBuf>>,
    ) -> Result<PathBuf> {
        let config_dir = ConfinuumConfig::get_dir().context("Could not get config dir")?;
        let files_dir = config_dir.join(&entry.name);

        let canonicalized = files
            .iter()
            .map(|x| {
                x.canonicalize()
                    .map_err(|e| anyhow!("Failed to canonicalize: {}", e))
            })
            .collect::<Result<Vec<PathBuf>>>()?;
        if base.is_none() {
            let prev_entry_files = entry
                .files
                .iter()
                .map(|f| {
                    entry
                        .target_dir
                        .as_ref()
                        .unwrap()
                        .join(&entry.name)
                        .join(&f)
                })
                .collect::<Vec<_>>();
            let all = prev_entry_files.iter().chain(canonicalized.iter());
            base = Some(
                common_path_all(all.map(|x| x.as_path()))
                    .ok_or(anyhow!("Could not find common base path"))?,
            );
            entry.target_dir = Some(base.clone().unwrap());
            /* if entry.target_dir.is_some() && entry.target_dir != base {
                return Err(anyhow!(
                    "Target directory {:?} does not match base path {:?}! All files in a config entry must share a common base path (such as ~/.config/nvim/), so that they can be properly placed in that directory.",
                    entry.target_dir,
                    base
                ));
            } else if entry.target_dir.is_none() {
                entry.target_dir = Some(base.clone().unwrap());
            } */
        }

        // First pass, collect all files and copy them to the config directory
        let mut new_files = vec![];
        for file in canonicalized {
            if !file.exists() {
                return Err(anyhow!("File does not exist: {:?}", file));
            }
            if file.is_dir() {
                if file.file_name().unwrap() == ".git" {
                    continue;
                }
                let entries = file
                    .read_dir()?
                    .filter_map(|x| if let Ok(x) = x { Some(x.path()) } else { None })
                    .collect::<Vec<_>>();
                Self::add_files_recursive(entry, entries, base.clone(), result_files)?;
            } else {
                let source_path = files_dir.join(
                    file.strip_prefix(&base.clone().unwrap()).with_context(|| {
                        format!(
                            "Could not strip prefix {} from {}",
                            base.as_ref().unwrap().display(),
                            file.display()
                        )
                    })?,
                );
                let parent_folder = source_path.parent().ok_or(anyhow!(
                    "Could not get parent folder for file: {:?}",
                    source_path
                ))?;
                if !parent_folder.exists() {
                    std::fs::create_dir_all(parent_folder).with_context(|| {
                        format!("Could not create dirs {}", parent_folder.display())
                    })?;
                }

                let repo_rel_source_path = source_path
                    .strip_prefix(&config_dir.join(&entry.name))
                    .with_context(|| {
                        format!(
                            "Could not strip prefix {} from {}",
                            &config_dir.display(),
                            &source_path.display()
                        )
                    })?
                    .to_path_buf();
                new_files.push(repo_rel_source_path.clone());
                std::fs::copy(&file, &source_path).with_context(|| {
                    format!(
                        "Could not copy {} to {}",
                        file.display(),
                        source_path.display()
                    )
                })?;
            }
        }

        // Files used to be symlinked here, but that was moved to
        //    the deploy function to be used in commands where needed.

        // Then add the new files to the entry and result files
        if let Some(result_files) = result_files {
            result_files.extend(new_files.iter().cloned());
        }
        entry.files.extend(new_files);
        Ok(base.unwrap())
    }

    pub fn exists() -> Result<bool> {
        let config_path = Self::get_path()?;
        if config_path.is_dir() {
            return Err(anyhow!(
                "Config file is a directory. Please remove it and try again."
            ));
        }
        Ok(config_path.exists() && config_path.is_file())
    }

    pub fn get_path() -> Result<PathBuf> {
        Ok(PathBuf::from(var("HOME")?).join(".config/confinuum/config.toml"))
    }

    pub fn get_dir() -> Result<PathBuf> {
        Ok(PathBuf::from(var("HOME")?).join(".config/confinuum"))
    }

    pub fn load() -> Result<ConfinuumConfig> {
        if !Self::exists()? {
            return Err(anyhow!(
                "Config file does not exist. Run `confinuum init` to create one."
            ));
        }
        let config_str = std::fs::read_to_string(Self::get_path()?)
            .context("Could not load confinuum config")?;
        let mut config: ConfinuumConfig =
            toml::from_str(&config_str).context("Could not parse confinuum config")?;
        config.entries.iter_mut().for_each(|(name, entry)| {
            entry.name = name.to_string();
        });
        Ok(config)
    }

    /// Save the config to disk (will overwrite existing config)
    pub fn save(&self) -> Result<()> {
        let config_path = Self::get_path()?;
        let config_str = toml::to_string_pretty(self)?;
        let conf_dir = ConfinuumConfig::get_dir()?;
        if !conf_dir.exists() {
            std::fs::create_dir_all(conf_dir)?;
        }
        std::fs::write(config_path, config_str)?;
        Ok(())
    }
}
