//! Author: Will Hopkins <willothyh@gmail.com>
//! Description: A simple CLI tool for managing program configurations across multiple machines.
//! License: MIT
#![cfg(not(windows))]

use std::io::stdout;

mod cli;
mod commands;
mod config;
mod git;
mod util;

// TODO: Allow for an entry to contain submodules / be a submodule

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if let Err(e) = cli::Cli::run().await {
        crossterm::execute!(
            stdout(),
            crossterm::cursor::MoveToColumn(0),
            crossterm::terminal::Clear(crossterm::terminal::ClearType::CurrentLine),
            crossterm::cursor::Show
        )?;
        return Err(e);
    }
    crossterm::execute!(std::io::stdout(), crossterm::cursor::Show).unwrap();

    Ok(())
}

#[cfg(windows)]
fn main() {
    panic!("This program does not support Windows.");
}
