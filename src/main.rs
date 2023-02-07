use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf};

#[derive(Debug, Deserialize, Serialize, Parser)]
pub struct ConfigEntry {
    pub name: String,
    pub dir: PathBuf,
    pub repo: String,
}

#[derive(Debug, Parser)]
#[command(author, version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Check,
    Add {
        name: String,
        dir: PathBuf,
        repo: String,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = r#"
        [test]
        name = "test"
        dir = "/tmp/test"
        repo = "https://github.com/username/test.git"
    "#;

    let config: HashMap<String, ConfigEntry> = toml::from_str(config)?;

    Ok(())
}
