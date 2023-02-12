use anyhow::{anyhow, Context, Result};

use crate::config::ConfinuumConfig;

pub fn deploy(name: Option<impl Into<String>>) -> Result<()> {
    let config = ConfinuumConfig::load()?;
    let config_dir = ConfinuumConfig::get_dir().context("Could not get config dir")?;
    let name: Option<String> = name.map(|n| n.into());
    if let Some(name) = &name {
        if !config.entries.contains_key(name) {
            return Err(anyhow!("No entry named {} found", name));
        }
    }

    let res = config
        .entries
        .iter()
        .filter_map(|(entry_name, entry)| {
            if let Some(name) = &name {
                if entry_name == name && entry.files.len() > 0 && entry.target_dir.is_some() {
                    Some(entry)
                } else {
                    None
                }
            } else {
                if entry.files.len() > 0 && entry.target_dir.is_some() {
                    Some(entry)
                } else {
                    None
                }
            }
        })
        .try_for_each(|entry| -> Result<()> {
            let target_dir = entry.target_dir.as_ref().unwrap();
            entry.files.iter().try_for_each(|file| -> Result<()> {
                let target_path = target_dir.join(&file);
                let source_path = config_dir.join(&entry.name).join(file);
                if !source_path.exists() {
                    return Err(anyhow!(
                        "File {} does not exist in configs",
                        source_path.display()
                    ));
                }
                if target_path.exists() {
                    if target_path.is_symlink() && target_path.read_link()? == source_path {
                        // If the file is already a symlink to the correct place, do nothing
                        return Ok(());
                    }
                    std::fs::remove_file(&target_path)
                        .with_context(|| format!("Cannot remove file {}", target_path.display()))?;
                }
                std::os::unix::fs::symlink(&source_path, &target_path).with_context(|| {
                    format!(
                        "Could not symlink {} to {}",
                        source_path.display(),
                        target_path.display()
                    )
                })?;

                Ok(())
            })
        });
    if res.is_err() {
        // If there was an error, undo the symlinks, return the files to their original locations, and return the error
        config
            .entries
            .iter()
            .filter_map(|(entry_name, entry)| {
                if let Some(name) = &name {
                    if entry_name == name && entry.files.len() > 0 && entry.target_dir.is_some() {
                        Some(entry)
                    } else {
                        None
                    }
                } else {
                    if entry.files.len() > 0 && entry.target_dir.is_some() {
                        Some(entry)
                    } else {
                        None
                    }
                }
            })
            .try_for_each(|entry| -> Result<()> {
                let entry_name = &entry.name;
                let target_dir = entry.target_dir.as_ref().unwrap();

                println!("Error symlinking files, reverting changes...");
                entry.files.iter().try_for_each(|file| -> Result<()> {
                    let target_path = target_dir.join(&file);
                    if !target_path.exists() {
                        std::fs::copy(&config_dir.join(&entry_name).join(&file), &target_path)
                            .with_context(|| {
                                format!(
                                    "Could not copy {} to {}",
                                    file.display(),
                                    target_path.display()
                                )
                            })?;
                    } else if target_path.is_symlink() && target_path.read_link()? == *file {
                        std::fs::remove_file(&target_path).with_context(|| {
                            format!("Could not remove {}", target_path.display())
                        })?;
                        std::fs::copy(&config_dir.join(&entry_name).join(&file), &target_path)
                            .with_context(|| {
                                format!(
                                    "Could not copy {} to {}",
                                    config_dir.join(&entry_name).join(&file).display(),
                                    target_path.display()
                                )
                            })?;
                    }
                    Ok(())
                })?;

                Ok(())
            })?;
    }

    Ok(())
}

pub fn undeploy(name: Option<impl Into<String>>) -> Result<()> {
    let config = ConfinuumConfig::load()?;
    let config_dir = ConfinuumConfig::get_dir()?;
    let name: Option<String> = name.map(|n| n.into());
    if let Some(name) = &name {
        if !config.entries.contains_key(name) {
            return Err(anyhow!("No entry named {} found", name));
        }
    }

    config
        .entries
        .iter()
        .filter_map(|(entry_name, entry)| {
            if let Some(name) = &name {
                if entry_name == name && entry.files.len() > 0 && entry.target_dir.is_some() {
                    Some(entry)
                } else {
                    None
                }
            } else {
                if entry.files.len() > 0 && entry.target_dir.is_some() {
                    Some(entry)
                } else {
                    None
                }
            }
        })
        .try_for_each(|entry| -> Result<()> {
            let entry_name = &entry.name;
            let target_dir = entry.target_dir.as_ref().unwrap();
            entry
                .files
                .iter()
                .map(|file| {
                    (
                        target_dir.join(file),
                        config_dir.join(entry_name).join(file),
                    )
                })
                .try_for_each(|(symlink, expected_target)| -> Result<()> {
                    if symlink.exists() && symlink.is_symlink() {
                        if let Ok(link_target) = symlink.read_link() {
                            if link_target == expected_target {
                                std::fs::remove_file(symlink)?;
                            }
                        }
                    }
                    Ok(())
                })?;
            Ok(())
        })?;

    Ok(())
}
