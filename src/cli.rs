//! Command line interface for confinuum

use std::{
    borrow::Cow,
    cell::RefCell,
    fs::{self, File},
    io::{BufWriter, Write},
    path::PathBuf,
    rc::Rc,
};

use anyhow::{anyhow, Result};
use clap::{error::ErrorKind, CommandFactory, Parser, Subcommand, ValueHint};
use clap_complete::Shell;
use spinoff::{spinners::SpinnerFrames, Color, Spinner};

use crate::{commands, github};

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

#[derive(Debug, Subcommand)]
#[command(about, author, version, arg_required_else_help = true)]
pub enum EntryCommand {
    #[command(about = "Create a new config entry", long_about = None)]
    Create {
        /// Files to add to the config entry (optional, you can add files later)
        #[clap(value_hint = ValueHint::FilePath)]
        files: Option<Vec<PathBuf>>,
        /// Push the new config entry to the remote repo(s) after creating it, instead of waiting for a manual push (without this flag the change(s) will be committed locally but not pushed)
        #[clap(short = 'p', long)]
        push: bool,
    },
    #[command(about = "Delete the config entry (files will be restored to their original locations)", long_about = None)]
    Delete {
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
    #[command(about = "List files in the config entry", long_about = None)]
    Show,
    #[command(about = "Check if the config entry is up to date", long_about = None)]
    Check {
        /// Print the diff between the local and remote config files
        #[arg(short = 'd', long)]
        print_diff: bool,
    },
    #[command(about = "Add one or more files to an existing config entry", long_about = None)]
    #[command(visible_alias = "add")]
    AddFiles {
        #[clap(value_hint = ValueHint::FilePath)]
        files: Vec<PathBuf>,
        /// Push new files to the remote repo immediately, instead of waiting for a manual push (without this flag the change(s) will be committed locally but not pushed)
        #[clap(short = 'p', long)]
        push: bool,
    },
    #[command(about = "Remove one or more files from an existing config entry (files will be restored to their original locations)", long_about = None)]
    #[command(visible_alias = "rm", visible_alias = "remove")]
    RemoveFiles {
        #[clap(value_hint = ValueHint::FilePath)]
        files: Vec<PathBuf>,
        /// Don't ask for confirmation before removing the file(s)
        #[clap(short = 'y', long)]
        no_confirm: bool,
        #[clap(short = 'f', long)]
        /// Don't return files to their original locations, just delete them
        no_replace_files: bool,
        /// Push changes to the remote repo instead of waiting for a manual push (without this flag the change(s) will be committed locally but not pushed)
        #[clap(short = 'p', long)]
        push: bool,
    },
}

#[derive(Debug, Subcommand)]
#[command(about, author, version, arg_required_else_help = true)]
pub enum UtilCommand {
    #[command(about = "Generate manpages for confinuum")]
    Mangen {
        /// Output directory
        #[clap(value_hint = ValueHint::FilePath)]
        output: PathBuf,
    },
    #[command(about = "Generate completions for confinuum")]
    Completions {
        #[arg(required = true)]
        shell: Shell,
        /// Output file (optional, if not specified the completion will be printed to stdout)
        #[clap(value_hint = ValueHint::FilePath)]
        output: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
#[command(
    about,
    author,
    version,
    arg_required_else_help = true,
    display_order = 0
)]
pub enum Command {
    #[command(about = "Initialize the confinuum config file", long_about = None)]
    Init {
        /// Initialize from git repo containing an existing confinuum config
        #[arg(long, value_hint=ValueHint::Url)]
        git: Option<String>,
        /// Force overwrite of config file if it already exists
        #[clap(short, long)]
        force: bool,
    },
    #[command(about = "Create, modify and view entries", long_about = None)]
    Entry {
        /// Name of the config entry
        name: String,
        /// Action to perform on the entry
        #[command(subcommand)]
        command: EntryCommand,
    },
    #[command(about = "List all config entries", long_about = None)]
    #[command(visible_alias = "ls")]
    List,
    #[command(about = "Push config changes to remote repo", long_about = None)]
    Push,
    #[command(about = "Check for config updates", long_about = None)]
    #[command(visible_alias = "?")]
    Check {
        /// Print the diff between the local and remote config files
        #[arg(short = 'd', long)]
        print_diff: bool,
        /// Check for updates for a specific config entry (optional)
        name: Option<String>,
    },
    #[command(name="update", about = "Update config from the remote repo", long_about = None)]
    Update,
    #[command(name = "redeploy", about = "Redeploy all configs", long_about = None)]
    Redeploy,
    #[command(about = "Utility commands", long_about = None)]
    Util {
        #[command(subcommand)]
        command: UtilCommand,
    },
}

impl Cli {
    pub async fn run() -> Result<()> {
        let args = match Self::try_parse() {
            Ok(args) => args,
            Err(e) => match e.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                    println!("{}", e);
                    return Ok(());
                }
                _ => return Err(anyhow!("{}", e)),
            },
        };
        let github = github::Github::new().await?;

        match args.command {
            Command::Init { git, force } => commands::init(git, force, &github).await,
            Command::Entry { name, command } => match command {
                EntryCommand::Create { files, push } => {
                    commands::new(name, files, push, &github).await
                }
                EntryCommand::Delete {
                    no_confirm,
                    no_replace_files,
                    push,
                } => commands::delete(name, no_confirm, no_replace_files, push, &github).await,
                EntryCommand::Show => commands::show(name),
                EntryCommand::Check { print_diff } => commands::check(print_diff, Some(name)),
                EntryCommand::AddFiles { files, push } => {
                    commands::add(name, files, push, &github).await
                }
                EntryCommand::RemoveFiles {
                    files,
                    no_confirm,
                    no_replace_files,
                    push,
                } => {
                    commands::remove(name, files, no_confirm, no_replace_files, push, &github).await
                }
            },
            Command::List => commands::list(),
            Command::Push => commands::push(),
            Command::Check { print_diff, name } => commands::check(print_diff, name),
            Command::Update => commands::update(),
            Command::Redeploy => commands::redeploy(),
            Command::Util { command } => match command {
                UtilCommand::Mangen { output } => {
                    if output.is_file() {
                        return Err(anyhow!(
                            "{} is a file! Mangen output must be a directory",
                            output.display()
                        ));
                    }
                    if !output.exists() {
                        fs::create_dir_all(&output)?;
                    }
                    let cmd = Cli::command();

                    let confinuum_man_path = output.join("confinuum.1");
                    let mut writer = BufWriter::new(File::create(confinuum_man_path)?);
                    let man = clap_mangen::Man::new(cmd.clone());
                    man.render(&mut writer)?;
                    writer.flush()?;

                    for subcomand in cmd.get_subcommands() {
                        let subcmd_man = clap_mangen::Man::new(subcomand.clone());
                        let path = output.join(format!("confinuum-{}.1", subcomand.get_name()));
                        let mut writer = BufWriter::new(File::create(path)?);
                        subcmd_man.render(&mut writer)?;
                        writer.flush()?;
                    }

                    Ok(())
                }
                UtilCommand::Completions { shell, output } => {
                    let mut out: BufWriter<Box<dyn std::io::Write>> = if let Some(output) = output {
                        if !output.exists() {
                            fs::create_dir_all(&output.parent().unwrap())?;
                        }
                        BufWriter::new(Box::new(File::create(output)?))
                    } else {
                        BufWriter::new(Box::new(std::io::stdout()))
                    };
                    clap_complete::generate(shell, &mut Cli::command(), "confinuum", &mut out);
                    out.flush()?;
                    Ok(())
                }
            },
        }
    }
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
