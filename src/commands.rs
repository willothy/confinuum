//! Author: Will Hopkins <willothyh@gmail.com>
//! commands.rs: Command implementations for confinuum

use std::{cell::RefCell, collections::HashSet, path::PathBuf, rc::Rc};

use anyhow::{anyhow, Context, Result};
use crossterm::style::Stylize;
use dialoguer::{theme::ColorfulTheme, Select};
use git2::{DiffFormat, DiffOptions, Direction, FetchOptions, IndexAddOption, Repository};
use git_url_parse::GitUrl;
use spinoff::{spinners, Color, Spinner};

use crate::{
    cli::{CreateSharedSpinner, SharedSpinner},
    config::{ConfigEntry, ConfinuumConfig},
    git::{self, Github, RepoCreateInfo, RepoExtensions},
};

/// Initialize the confinuum config file
pub async fn init(git: Option<String>, force: bool, github: &Github) -> Result<()> {
    if ConfinuumConfig::exists()? && !force {
        return Err(anyhow::anyhow!(
            "Config file already exists. Use --force to overwrite."
        ));
    }
    // Create config directory if it doesn't exist
    let config_path = ConfinuumConfig::get_path().context("Could not get config path")?;
    let config_dir = match ConfinuumConfig::get_dir().context("Could not get config dir")? {
        dir if dir.exists() => dir.to_path_buf(),
        nonexistent => {
            std::fs::create_dir_all(&nonexistent).context("Could not create directory")?;
            nonexistent.to_path_buf()
        }
    };

    let mut spinner: Option<Rc<RefCell<Spinner>>> = None;
    if let Some(git_url) = git {
        Repository::clone(&git_url, config_dir).context(format!("Failed to clone {}", git_url))?;
        eprintln!("TODO: Setup configs");
    } else {
        let items = vec![
            "Create a new GitHub repository for me",
            "I'll create my own remote repository",
        ];
        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Welcome to Confinuum! How would you like to host your configs?")
            .items(&items)
            .default(0)
            .interact_opt()?
            .ok_or(anyhow!("No selection made, cancelling."))?;

        let remote_url = match selection {
            0 => {
                let repo_info = RepoCreateInfo {
                    name: "confinuum-config".to_owned(),
                    description: "My confinuum config".to_owned(),
                    private: true,
                    is_template: false,
                    opt: None,
                };
                let repo = github.create_repo(repo_info).await?;
                if let Some(remote) = repo.ssh_url {
                    GitUrl::parse(&remote.to_string()).map_err(|e| {
                        anyhow::anyhow!(format!("Could not parse {} as a git url: {}", remote, e))
                    })?
                } else {
                    return Err(anyhow!("No URL found for created repository"));
                }
            }
            1 => {
                let remote_url: GitUrl = dialoguer::Input::with_theme(&ColorfulTheme::default())
                    .with_prompt("Enter the URL of your remote repository")
                    .interact()?;
                if remote_url.to_string().is_empty() {
                    return Err(anyhow!("No URL provided, cancelling."));
                }
                remote_url
            }
            _ => unreachable!("Invalid selection made"),
        };

        let mut init_opt = git2::RepositoryInitOptions::new();
        init_opt.initial_head("main");
        init_opt.description("My confinuum config");
        init_opt.no_reinit(!force);
        let repo = Repository::init_opts(&config_dir, &init_opt)
            .context("Failed to initialize config git repository")?;

        let mut remote = repo.remote("origin", &remote_url.to_string())?;
        crossterm::execute!(std::io::stdout(), crossterm::cursor::Hide)?;
        spinner = Some(Spinner::new_shared(
            spinners::Dots9,
            "Connecting to remote 'origin'",
            Color::Blue,
        ));
        let spinner = spinner.as_ref().unwrap();

        // TODO: Figure out how to make sure the remote is empty
        /* let mut fetchopt = git2::FetchOptions::new();
        fetchopt.update_fetchhead(false);
        fetchopt.remote_callbacks(git::construct_callbacks(spinner.clone()));
        spinner
            .borrow_mut()
            .update_text("Ensuring new remote is empty");
        remote
            .fetch(
                &["refs/heads/main:refs/heads/main"],
                Some(&mut fetchopt),
                None,
            )
            .with_context(|| "Failed to fetch from new origin")?;
        remote.disconnect()?;
        // ensure fetch_head is empty
        let fetch_head = repo.find_reference("FETCH_HEAD").with_context(|| {
            crossterm::execute!(
                std::io::stdout(),
                crossterm::cursor::Show,
                MoveToColumn(0),
                Clear(crossterm::terminal::ClearType::CurrentLine)
            )
            .ok();
            "Failed to find fetch_head"
        })?;
        if fetch_head.target().is_some() {
            return Err(anyhow!("Remote is not empty, aborting."));
        }
        spinner.update_text("Remote is empty, continuing..."); */

        std::fs::write(
            &config_path,
            toml::to_string_pretty(&ConfinuumConfig::default())?,
        )?;
        let gitignore_path = config_dir.join(".gitignore");
        std::fs::write(&gitignore_path, "hosts.toml\n")?;
        let mut index = repo.index()?;

        let config_path_rel = pathdiff::diff_paths(&config_path, &config_dir)
            .context("Could not get relative path")?;
        index
            .add_path(&config_path_rel)
            .with_context(|| format!("Could not add path {}", config_path_rel.display()))?;
        let gitignore_path_ref = pathdiff::diff_paths(&gitignore_path, &config_dir)
            .context("Could not get relative path")?;
        index
            .add_path(&gitignore_path_ref)
            .with_context(|| format!("Could not add path {}", gitignore_path_ref.display()))?;

        let oid = index.write_tree()?;
        let sig = github.get_user_signature().await?;
        //let parent_commit = repo.find_last_commit()?;
        let tree = repo.find_tree(oid)?;
        let message = "Initial confinuum commit! 🎉";
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[])?;
        // TODO: Allow signing commits
        // repo.commit_signed(commit_content, signature, signature_field)

        let mut pushopt = git2::PushOptions::new();
        pushopt.remote_callbacks(git::construct_callbacks(spinner.clone()));
        spinner
            .borrow_mut()
            .update_text("Pushing changes to remote");
        remote.push(&["refs/heads/main:refs/heads/main"], Some(&mut pushopt))?;
    }
    if let Some(spinner) = spinner {
        spinner.success("Changes pushed successfully.");
        crossterm::execute!(std::io::stdout(), crossterm::cursor::Show)?;
    }

    Ok(())
}

/// Add a new config entry
pub async fn new(name: String, files: Option<Vec<PathBuf>>, github: &Github) -> Result<()> {
    // TODO: Revert files on error
    // Check for remote changes before adding files
    let config_dir = ConfinuumConfig::get_dir()?;
    let repo = Repository::open(&config_dir)?;
    let mut remote = repo.find_remote("origin")?;
    let spinner = Spinner::new_shared(
        spinners::Dots9,
        "Connecting to remote 'origin'",
        Color::Blue,
    );
    remote.connect_auth(
        Direction::Fetch,
        Some(git::construct_callbacks(spinner.clone())),
        None,
    )?;
    spinner.update_text("Checking for changes on remote");
    let mut fetch_opt = FetchOptions::new();
    fetch_opt.update_fetchhead(true);
    fetch_opt.remote_callbacks(git::construct_callbacks(spinner.clone()));
    remote
        .fetch(&["main"], Some(&mut fetch_opt), None)
        .context("Failed to fetch from remote 'origin'")?;
    let fetch_head = repo.find_reference("FETCH_HEAD")?;
    let fetch_commit = repo.reference_to_annotated_commit(&fetch_head)?;
    let analysis = repo.merge_analysis(&[&fetch_commit])?;
    remote.disconnect()?;
    if analysis.0.is_up_to_date() {
        spinner.success("No changes found on remote");
    } else {
        spinner.fail("Changes found on remote");
        return Err(anyhow!(
            "Changes found on remote. Please pull them before adding files."
        ));
    }

    let mut config = ConfinuumConfig::load()?;
    if config.entries.contains_key(&name) {
        return Err(anyhow!(
            "Entry named {} already exists! Use the `add` and `remove` subcommands to add or remove files from it.",
            name
        ));
    }

    config.entries.insert(
        name.clone(),
        ConfigEntry {
            name: name.clone(),
            files: HashSet::new(),
        },
    );
    let entry = config.entries.get_mut(&name).unwrap();
    let mut result_files = HashSet::new();
    if let Some(files) = files {
        ConfinuumConfig::add_files_recursive(entry, files, None, &mut Some(&mut result_files))?;
    }

    let mut index = repo.index()?;
    /* for file in &result_files {
        if !file.source_path.exists() {
            return Err(anyhow!(
                "File {} does not exist. Your confinuum config may be corrupted.",
                file.source_path.display()
            ));
        }
        let repo_relative = file.source_path.strip_prefix(&config_dir)?;
        index
            .add_path(repo_relative)
            .with_context(|| format!("Could not add path {}", repo_relative.display()))?;
    } */
    let mut imp = |path: &std::path::Path, data: &[u8]| {
        // Do Something
        if path.starts_with(".git") {
            return 1; // skip .git/
        }
        return 0;
    };
    index.add_all(["*"], IndexAddOption::DEFAULT, Some(&mut imp))?;
    let oid = index.write_tree()?;
    let parent_commit = repo.find_last_commit()?;
    let sig = github.get_user_signature().await?;
    let tree = repo.find_tree(oid)?;
    let message = format!(
        "Added configs for `{}`{}\n\nNew files:\n{}",
        name,
        if result_files.is_empty() {
            "".to_owned()
        } else {
            format!(" with {} files", result_files.len())
        },
        result_files
            .iter()
            .map(|f| f.source_path.display().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    );

    repo.commit(Some("HEAD"), &sig, &sig, &message, &tree, &[&parent_commit])?;

    crossterm::execute!(std::io::stdout(), crossterm::cursor::Hide)?;
    let spinner = Spinner::new_shared(
        spinners::Dots9,
        "Connecting to remote 'origin'",
        Color::Blue,
    );
    {
        remote.connect_auth(
            git2::Direction::Push,
            Some(git::construct_callbacks(spinner.clone())),
            None,
        )?;
        let mut pushopt = git2::PushOptions::new();
        pushopt.remote_callbacks(git::construct_callbacks(spinner.clone()));
        spinner.update_text("Pushing changes to remote");
        remote.push(&["refs/heads/main:refs/heads/main"], Some(&mut pushopt))?;
        // Scope to ensure that all references to spinner are dropped before we call success
    }
    spinner.success("Changes pushed successfully.");

    Ok(())
}

/// Remove a config entry (files will be restored to their original locations unless no_replace_files is set)
pub fn delete(name: String, no_confirm: bool, no_replace_files: bool) -> Result<()> {
    todo!();
    let mut config = ConfinuumConfig::load()?;
    if !config.entries.contains_key(&name) {
        return Err(anyhow!("No entry named {}", name));
    }
    config.entries.remove(&name);
    config.save()?;
    Ok(())
}

/// Add files to an existing config entry
pub fn add(name: String, files: Vec<PathBuf>) -> Result<()> {
    /* let mut config = ConfinuumConfig::load()?;
    if config.entries.contains_key(&name) {
        return Err(anyhow!(
            "Entry named {} already exists! Use the `edit` subcommand to modify it.",
            name
        ));
    }
    config
        .entries
        .insert(name.clone(), crate::config::ConfigEntry { name, dir, repo });
    config.save()?; */
    //Ok(())
    todo!();
}

pub fn remove(name: String, files: Vec<PathBuf>, no_confirm: bool) -> Result<()> {
    todo!()
}

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

pub fn source(name: Option<String>) -> Result<()> {
    todo!()
}

pub fn push(name: Option<String>) -> Result<()> {
    todo!()
}

pub fn check(print_diff: bool, name: Option<String>) -> Result<()> {
    let config_dir = ConfinuumConfig::get_dir()?;
    if !config_dir.exists() {
        return Err(anyhow!("Config directory does not exist"));
    }
    let repo =
        Repository::open(config_dir).context("Failed to open config directory as a git repo")?;
    crossterm::execute!(std::io::stdout(), crossterm::cursor::Hide)?;
    let spinner = Spinner::new_shared(
        spinners::Dots9,
        "Connecting to remote 'origin'",
        Color::Blue,
    );

    let analysis = {
        let mut remote = repo
            .find_remote("origin")
            .context("Failed to find remote named 'origin'")?;
        remote.connect_auth(
            Direction::Fetch,
            Some(git::construct_callbacks(spinner.clone())),
            None,
        )?;
        let mut fetch_opt = FetchOptions::new();
        fetch_opt.update_fetchhead(true);

        fetch_opt.remote_callbacks(git::construct_callbacks(spinner.clone()));

        remote
            .fetch(&["main"], Some(&mut fetch_opt), None)
            .context("Failed to fetch from remote 'origin'")?;

        let fetch_head = repo.find_reference("FETCH_HEAD")?;
        let fetch_commit = repo.reference_to_annotated_commit(&fetch_head)?;
        //let head_commit = repo.reference_to_annotated_commit(&head)?;
        let analysis = repo.merge_analysis(&[&fetch_commit])?;

        if print_diff {
            let head = repo.head()?;
            let head_tree = head.peel_to_tree()?;
            let fetch_tree = fetch_head.peel_to_tree()?;
            let mut diff_opt = DiffOptions::default();
            let diff =
                repo.diff_tree_to_tree(Some(&head_tree), Some(&fetch_tree), Some(&mut diff_opt))?;
            git::print_diff(&diff, DiffFormat::Patch)?;
        }

        analysis
    };

    if analysis.0.is_up_to_date() {
        spinner.success("Config is up to date");
    } else {
        spinner.warn(&format!(
            "Config is out of date! Run {} to sync changes.",
            "confinuum update".bold()
        ));
    }
    crossterm::execute!(std::io::stdout(), crossterm::cursor::Show)?;

    Ok(())
}

pub fn update() -> Result<()> {
    todo!()
}
