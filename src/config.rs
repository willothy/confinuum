//! Author: Will Hopkins <willothyh@gmail.com>
//! config.rs: Configuration file handling for confinuum

use std::{
    collections::{HashMap, HashSet},
    env::var,
    path::PathBuf,
};

use anyhow::{anyhow, Result};
use common_path::common_path_all;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct ConfigEntry {
    #[serde(skip)]
    pub name: String,
    pub files: HashSet<ConfigFile>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Hash, Clone)]
pub struct ConfigFile {
    /// The path to the file on the local machine, relative to the home directory
    pub target_path: PathBuf,
    /// The path to the file in the git repo, relative to the root of the repo
    pub source_path: PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Confinuum {
    pub git_protocol: Option<GitProtocol>,
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
        result_files: &mut Option<&mut HashSet<ConfigFile>>,
    ) -> Result<()> {
        let config_dir = ConfinuumConfig::get_dir()?;
        //let home_dir = var("HOME").map_err(|_| anyhow!("Could not get home directory"))?;
        let files_dir = config_dir.join(&entry.name);

        let canonicalized = files
            .iter()
            .map(|x| x.canonicalize().map_err(|e| anyhow!("{}", e)))
            .collect::<Result<Vec<PathBuf>>>()?;
        if base.is_none() {
            base = Some(
                common_path_all(canonicalized.iter().map(|x| x.as_path()))
                    .ok_or(anyhow!("Could not find common base path"))?,
            );
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
                let source_path = files_dir.join(file.strip_prefix(&base.clone().unwrap())?);
                let parent_folder = source_path.parent().ok_or(anyhow!(
                    "Could not get parent folder for file: {:?}",
                    source_path
                ))?;
                if !parent_folder.exists() {
                    std::fs::create_dir_all(parent_folder)?;
                }
                //entry.files.insert();
                new_files.push(ConfigFile {
                    target_path: file.clone(),
                    source_path: source_path.clone(),
                });
                std::fs::copy(file, source_path)?;
            }
        }

        // Second pass, remove old files and symlink in configs from the repo
        // Do this separately to ensure that if there are errors copying files, we don't remove the old ones and lose them
        let mut err_undo = Ok(());
        for file in new_files.iter() {
            std::fs::remove_file(&file.target_path)?;
            let link = std::os::unix::fs::symlink(&file.source_path, &file.target_path);
            if link.is_err() {
                err_undo = link;
                break;
            }
        }
        // If there was an error, undo the symlinks, return the files to their original locations, and return the error
        if err_undo.is_err() {
            println!("Error symlinking files, reverting changes...");
            for file in new_files.iter() {
                if !file.target_path.exists() {
                    std::fs::copy(&file.source_path, &file.target_path)?;
                } else if file.target_path.is_symlink()
                    && file.target_path.read_link()? == file.source_path
                {
                    std::fs::remove_file(&file.target_path)?;
                    std::fs::copy(&file.source_path, &file.target_path)?;
                }
            }
            return err_undo.map_err(|e| anyhow!("{}", e));
        }
        // Then add the new files to the entry and result files
        if let Some(result_files) = result_files {
            result_files.extend(new_files.iter().cloned());
        }
        entry.files.extend(new_files);
        Ok(())
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
        let config_str = std::fs::read_to_string(Self::get_path()?)?;
        let mut config: ConfinuumConfig = toml::from_str(&config_str)?;
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

impl Default for ConfinuumConfig {
    fn default() -> Self {
        Self {
            confinuum: Some(Confinuum {
                git_protocol: Some(GitProtocol::Ssh),
            }),
            entries: HashMap::new(),
        }
    }
}
