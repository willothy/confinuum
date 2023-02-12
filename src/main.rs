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

// TODO: Allow for an entry to contain submodules or be a submodule

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Panic handler
    std::panic::set_hook(Box::new(|info| {
        crossterm::execute!(
            stdout(),
            crossterm::cursor::MoveToColumn(0),
            crossterm::terminal::Clear(crossterm::terminal::ClearType::CurrentLine),
            crossterm::cursor::Show
        )
        .unwrap();
        println!("\nThe program has panicked! Please report this to https://github.com/willothy/confinuum/issues");
        if let Some(location) = info.location() {
            let message = info
                .payload()
                .downcast_ref::<&str>()
                .unwrap_or(&"<could not get panic message>");
            println!("Panicked with \"{}\" at {}", message, location);
            println!("Backtrace:\n{}", std::backtrace::Backtrace::force_capture());
        }
    }));

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
