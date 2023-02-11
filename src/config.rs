//! Author: Will Hopkins <willothyh@gmail.com>
//! config.rs: Configuration file handling for confinuum

use std::{
    collections::{HashMap, HashSet},
    env::var,
    path::PathBuf,
};

use anyhow::{anyhow, Context, Result};
use common_path::common_path_all;
use email_address::EmailAddress;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct Confinuum {
    pub git_protocol: Option<GitProtocol>,
    pub git_user: Option<String>,
    pub git_email: Option<EmailAddress>,
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
    pub confinuum: Option<Confinuum>,
    #[serde(flatten)]
    pub entries: HashMap<String, ConfigEntry>,
}

impl ConfinuumConfig {
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
            base = Some(
                common_path_all(canonicalized.iter().map(|x| x.as_path()))
                    .ok_or(anyhow!("Could not find common base path"))?,
            );
            if entry.target_dir.is_some() && entry.target_dir != base {
                return Err(anyhow!(
                    "Target directory {:?} does not match base path {:?}! All files in a config entry must share a common base path (such as ~/.config/nvim/), so that they can be properly placed in that directory.",
                    entry.target_dir,
                    base
                ));
            } else if entry.target_dir.is_none() {
                entry.target_dir = Some(base.clone().unwrap());
            }
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

        // NOTE: Second pass moved to `deploy` function in `crate::util`
        // Second pass, remove old files and symlink in configs from the repo
        // Do this separately to ensure that if there are errors copying files, we don't remove the old ones and lose them
        /*         let mut err_undo = Ok(());
        let config_dir = ConfinuumConfig::get_dir().context("Could not get config dir")?;
        for file in new_files.iter() {
            let target_path = base.as_ref().unwrap().join(&file).canonicalize()?;
            std::fs::remove_file(&target_path)
                .with_context(|| format!("Cannot remove file {}", target_path.display()))?;
            let link =
                std::os::unix::fs::symlink(config_dir.join(&entry.name).join(file), &target_path)
                    .with_context(|| {
                        format!(
                            "Could not symlink {} to {}",
                            file.display(),
                            target_path.display()
                        )
                    });
            if link.is_err() {
                err_undo = link;
                break;
            }
        }
        // If there was an error, undo the symlinks, return the files to their original locations, and return the error
        if err_undo.is_err() {
            println!("Error symlinking files, reverting changes...");
            for file in new_files.iter() {
                let target_path = base.as_ref().unwrap().join(&file);
                if !target_path.exists() {
                    std::fs::copy(&file, &target_path).with_context(|| {
                        format!(
                            "Could not copy {} to {}",
                            file.display(),
                            target_path.display()
                        )
                    })?;
                } else if target_path.is_symlink() && target_path.read_link()? == *file {
                    std::fs::remove_file(&target_path)
                        .with_context(|| format!("Could not remove {}", target_path.display()))?;
                    std::fs::copy(&file, &target_path).with_context(|| {
                        format!(
                            "Could not copy {} to {}",
                            file.display(),
                            target_path.display()
                        )
                    })?;
                }
            }
            return Err(anyhow!("{}", err_undo.unwrap_err()));
        } */

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

    pub fn default_with_user(user: String, email: EmailAddress) -> Self {
        Self {
            confinuum: Some(Confinuum {
                git_protocol: Some(GitProtocol::Ssh),
                git_user: Some(user),
                git_email: Some(email),
            }),
            entries: HashMap::new(),
        }
    }
}

impl Default for ConfinuumConfig {
    fn default() -> Self {
        Self {
            confinuum: Some(Confinuum {
                git_protocol: Some(GitProtocol::Ssh),
                git_user: None,
                git_email: None,
            }),
            entries: HashMap::new(),
        }
    }
}
