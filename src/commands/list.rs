use crate::config::ConfinuumConfig;
use anyhow::Result;

pub fn list() -> Result<()> {
    let config = ConfinuumConfig::load()?;
    /* for (name, entry) in config.entries {
        println!(
            "{}: {}\n \u{21B3} {}",
            name,
            entry.dir.display(),
            entry.repo
        );
    } */
    todo!();
    Ok(())
}
