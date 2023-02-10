//! Author: Will Hopkins <willothyh@gmail.com>
//! cli.rs: Command line interface for confinuum

use std::{borrow::Cow, cell::RefCell, path::PathBuf, rc::Rc};

use anyhow::{anyhow, Result};
use clap::{error::ErrorKind, Parser, Subcommand};
use spinoff::{spinners::SpinnerFrames, Color, Spinner};

use crate::{commands, git};

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    about,
    long_about = None,
)]
#[command(propagate_version = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    pub async fn run() -> Result<()> {
        let args = match Self::try_parse() {
            Ok(args) => args,
            Err(e) => match e.kind() {
                ErrorKind::DisplayHelp
                | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
                | ErrorKind::DisplayVersion => {
                    println!("{}", e);
                    return Ok(());
                }
                e => return Err(anyhow!("Error: {:?}", e)),
            },
        };
        let github = git::Github::new().await?;

        match args.command {
            Command::Init { git, force } => commands::init(git, force, &github).await,
            Command::New { name, files, push } => commands::new(name, files, push, &github).await,
            Command::Delete {
                name,
                no_confirm,
                no_replace_files,
                push,
            } => commands::delete(name, no_confirm, no_replace_files, push, &github).await,
            Command::Add { name, files, push } => commands::add(name, files, push, &github).await,
            Command::Remove {
                name,
                files,
                no_confirm,
                push,
            } => commands::remove(name, files, no_confirm, push),
            Command::List => commands::list(),
            Command::Push { name } => commands::push(name),
            Command::Check { print_diff, name } => commands::check(print_diff, name),
            Command::Update => commands::update(),
        }
    }
}

#[derive(Debug, Subcommand)]
#[command(about, author, version)]
pub enum Command {
    #[command(about = "Initialize the confinuum config file", long_about = None)]
    Init {
        /// Initialize config from git repo
        #[arg(long)]
        git: Option<String>,
        /// Force overwrite of config file if it already exists
        #[clap(short, long)]
        force: bool,
    },
    #[command(about = "Add a new config entry", long_about = None)]
    New {
        /// Name of the config entry
        name: String,
        /// Files to add to the config entry (optional, you can add files later)
        files: Option<Vec<PathBuf>>,
        /// Push the new config entry to the remote repo(s) after creating it, instead of waiting for a manual push (without this flag the change(s) will be committed locally but not pushed)
        #[clap(short = 'p', long)]
        push: bool,
    },
    #[command(about = "Remove a config entry (files will be restored to their original locations unless no_replace_files is set)", long_about = None)]
    Delete {
        name: String,
        /// Don't ask for confirmation before deleting the entry
        #[clap(short = 'y', long)]
        no_confirm: bool,
        /// Don't return files to their original locations, just delete them along with the entry
        #[clap(short = 'f', long)]
        no_replace_files: bool,
        /// Push the deletion to the remote repo (without this flag the deletion will be committed locally but not pushed)
        #[clap(short = 'p', long)]
        push: bool,
    },
    #[command(about = "Add one or more files to an existing config entry", long_about = None)]
    Add {
        name: String,
        files: Vec<PathBuf>,
        /// Push new files to the remote repo immediately, instead of waiting for a manual push (without this flag the change(s) will be committed locally but not pushed)
        #[clap(short = 'p', long)]
        push: bool,
    },
    #[command(about = "Remove one or more files from an existing config entry", long_about = None)]
    Remove {
        name: String,
        files: Vec<PathBuf>,
        /// Don't ask for confirmation before removing the file(s)
        #[clap(short = 'y', long)]
        no_confirm: bool,
        /// Push changes to the remote repo instead of waiting for a manual push (without this flag the change(s) will be committed locally but not pushed)
        #[clap(short = 'p', long)]
        push: bool,
    },
    #[command(about = "List all config entries", long_about = None)]
    List,
    #[command(about = "Push config changes to remote repo(s)", long_about = None)]
    Push { name: Option<String> },
    #[command(about = "Check for config updates", long_about = None)]
    Check {
        /// Print the diff between the local and remote config files
        #[arg(short = 'd', long)]
        print_diff: bool,
        /// Check for updates for a specific config entry (optional)
        name: Option<String>,
    },
    #[command(name="update", about = "Update config from the remote repo", long_about = None)]
    Update,
}

pub trait CreateSharedSpinner {
    fn new_shared(
        frames: impl Into<SpinnerFrames>,
        message: impl Into<Cow<'static, str>>,
        color: Color,
    ) -> Rc<RefCell<Self>>;
}

impl CreateSharedSpinner for spinoff::Spinner {
    fn new_shared(
        frames: impl Into<SpinnerFrames>,
        message: impl Into<Cow<'static, str>>,
        color: Color,
    ) -> Rc<RefCell<Self>> {
        crossterm::execute!(std::io::stdout(), crossterm::cursor::Hide).ok();
        Rc::new(RefCell::new(Spinner::new(frames, message, color)))
    }
}

pub trait SharedSpinner {
    fn stop(self);
    fn stop_with_message(self, message: &str);
    fn success(self, message: &str);
    fn warn(self, message: &str);
    fn fail(self, message: &str);
    fn clear(self);
    fn update_text(&self, message: impl Into<Cow<'static, str>>);
}

impl SharedSpinner for Rc<RefCell<spinoff::Spinner>> {
    fn stop(self) {
        let unwrapped = Rc::try_unwrap(self);
        if let Ok(unwrapped) = unwrapped {
            unwrapped.into_inner().stop();
        }
        crossterm::execute!(std::io::stdout(), crossterm::cursor::Show).unwrap();
    }

    fn clear(self) {
        let unwrapped = Rc::try_unwrap(self);
        if let Ok(unwrapped) = unwrapped {
            unwrapped.into_inner().clear();
        }
        crossterm::execute!(std::io::stdout(), crossterm::cursor::Show).unwrap();
    }

    fn stop_with_message(self, message: &str) {
        let unwrapped = Rc::try_unwrap(self);
        if let Ok(unwrapped) = unwrapped {
            unwrapped.into_inner().stop_with_message(message);
        }
        crossterm::execute!(std::io::stdout(), crossterm::cursor::Show).unwrap();
    }

    fn success(self, message: &str) {
        let unwrapped = Rc::try_unwrap(self);
        if let Ok(unwrapped) = unwrapped {
            unwrapped.into_inner().success(message);
        }
        crossterm::execute!(std::io::stdout(), crossterm::cursor::Show).unwrap();
    }

    fn warn(self, message: &str) {
        let unwrapped = Rc::try_unwrap(self);
        if let Ok(unwrapped) = unwrapped {
            unwrapped.into_inner().warn(message);
        }
        crossterm::execute!(std::io::stdout(), crossterm::cursor::Show).unwrap();
    }

    fn fail(self, message: &str) {
        let unwrapped = Rc::try_unwrap(self);
        if let Ok(unwrapped) = unwrapped {
            unwrapped.into_inner().fail(message);
        }
        crossterm::execute!(std::io::stdout(), crossterm::cursor::Show).unwrap();
    }

    fn update_text(&self, message: impl Into<Cow<'static, str>>) {
        self.borrow_mut().update_text(message);
    }
}
