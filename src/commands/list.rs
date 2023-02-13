use crate::config::ConfinuumConfig;
use anyhow::Result;
use crossterm::style::Stylize;

pub(crate) fn list() -> Result<()> {
    let config = ConfinuumConfig::load()?;
    for (name, entry) in config.entries {
        if let Some(target_dir) = &entry.target_dir {
            println!(
                "{}: {} files\n\u{21B3} {}",
                name.bold().yellow(),
                entry.files.len(),
                target_dir.display()
            );
        } else {
            println!("{}: uninitialized", name.bold().yellow());
        }
    }
    Ok(())
}
