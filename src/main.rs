//! Author: Will Hopkins <willothyh@gmail.com>
//! Description: A simple CLI tool for managing program configurations across multiple machines.
//! License: MIT
#![cfg(not(windows))]

mod cli;
mod commands;
mod config;
mod git;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match cli::Cli::run().await {
        Ok(_) => todo!(),
        Err(e) => println!("Error: {}", e),
    }
    crossterm::execute!(std::io::stdout(), crossterm::cursor::Show).unwrap();
    Ok(())
}

#[cfg(windows)]
fn main() {
    panic!("This program does not support Windows.");
}
