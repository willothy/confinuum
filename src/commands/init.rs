use std::{cell::RefCell, rc::Rc};

use anyhow::{anyhow, Context, Result};
use dialoguer::{theme::ColorfulTheme, Select};
use git2::Repository;
use git_url_parse::GitUrl;
use spinoff::{spinners, Color, Spinner};

use crate::{
    cli::{CreateSharedSpinner, SharedSpinner},
    config::ConfinuumConfig,
    git::{self, Github, RepoCreateInfo},
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
