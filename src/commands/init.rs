use anyhow::{anyhow, Context, Result};
use dialoguer::{theme::ColorfulTheme, Select};
use git2::Repository;
use git_url_parse::GitUrl;
use spinoff::{spinners, Color, Spinner};

use crate::{
    cli::{CreateSharedSpinner, SharedSpinner},
    config::{ConfinuumConfig, GitProtocol, SignatureSource},
    git::{self, Github, RepoCreateInfo},
    util,
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

    // If user provided a git url, we can just clone it as it's already set up
    if let Some(git_url) = git {
        // Clone the repo
        // TODO: Ensure the clone contains a valid config file, and if so validate the entries
        Repository::clone(&git_url, config_dir).context(format!("Failed to clone {}", git_url))?;
        util::deploy(None::<&str>)?;
        return Ok(());
    }

    let items = vec![
        "Create a new GitHub repository for me",
        "I'll create my own remote repository",
    ];

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("How would you like to host your configs?")
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
            let spinner = Spinner::new(
                spinners::Dots9,
                "Creating repository".to_string(),
                Color::Blue,
            );
            let repo = github.create_repo(repo_info).await?;
            spinner.success(&format!("Created repository {}!", &repo.name));

            let protocol = dialoguer::Select::with_theme(&ColorfulTheme::default())
                .with_prompt("Which protocol would you like to use?")
                .items(&["SSH", "HTTPS"])
                .default(0)
                .interact()?;

            if protocol == 0 {
                if let Some(remote) = repo.ssh_url {
                    GitUrl::parse(&remote.to_string()).map_err(|e| {
                        anyhow::anyhow!(format!("Could not parse {} as a git url: {}", remote, e))
                    })?
                } else {
                    return Err(anyhow!("No URL found for created repository"));
                }
            } else {
                GitUrl::parse(&repo.url.to_string()).map_err(|e| {
                    anyhow::anyhow!(format!("Could not parse {} as a git url: {}", &repo.url, e))
                })?
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

    let git_protocol = match remote_url.scheme {
        git_url_parse::Scheme::Https => GitProtocol::Https,
        git_url_parse::Scheme::Ssh => GitProtocol::Ssh,
        unsupported => {
            return Err(anyhow!(
                "Git protocol {} is not yet supported by Confinuum :/",
                unsupported
            ))
        }
    };

    let signature_source = match dialoguer::Select::with_theme(&ColorfulTheme::default())
        .with_prompt("How would you like to sign your commits? Confinuum can source your name/email from you github account, or your git config.")
        .items(&["GitHub", "Git config"])
        .interact()? {
            0 => SignatureSource::Github,
            1 => SignatureSource::GitConfig,
            _ => unreachable!("Impossible selection made!"),
        };

    // Get the user's signature
    let signature = match signature_source {
        SignatureSource::Github => github
            .get_user_signature()
            .await
            .context("Could not fetch user signature from github")?,
        SignatureSource::GitConfig => {
            // allows users to set values in config if they don't exist
            git::gitconfig::get_user_sig_with_prompt()?
        }
    };

    let spinner = Spinner::new_shared(
        spinners::Dots9,
        "Welcome to confinuum! Setting things up",
        Color::Blue,
    );

    let mut init_opt = git2::RepositoryInitOptions::new();
    init_opt.initial_head("main");
    init_opt.description("My confinuum config");
    init_opt.no_reinit(!force);
    let repo = Repository::init_opts(&config_dir, &init_opt)
        .context("Failed to initialize config git repository")?;

    let mut remote = repo.remote("origin", &remote_url.to_string())?;

    // TODO: Figure out how to make sure the remote is empty
    std::fs::write(
        &config_path,
        toml::to_string_pretty(&ConfinuumConfig::init(git_protocol, signature_source))?,
    )?;
    let gitignore_path = config_dir.join(".gitignore");
    std::fs::write(&gitignore_path, "hosts.toml\n")?;
    let mut index = repo.index()?;

    let config_path_rel =
        pathdiff::diff_paths(&config_path, &config_dir).context("Could not get relative path")?;
    index
        .add_path(&config_path_rel)
        .with_context(|| format!("Could not add path {}", config_path_rel.display()))?;
    let gitignore_path_ref = pathdiff::diff_paths(&gitignore_path, &config_dir)
        .context("Could not get relative path")?;
    index
        .add_path(&gitignore_path_ref)
        .with_context(|| format!("Could not add path {}", gitignore_path_ref.display()))?;

    let oid = index.write_tree()?;

    //let parent_commit = repo.find_last_commit()?;
    let tree = repo.find_tree(oid)?;
    let message = "Initial confinuum commit! 🎉";
    repo.commit(Some("HEAD"), &signature, &signature, message, &tree, &[])?;
    // TODO: Allow signing commits
    // repo.commit_signed(commit_content, signature, signature_field)
    {
        // Scope ensures that the spinner is dropped before we clear it
        spinner
            .borrow_mut()
            .update_text("Pushing changes to remote");
        let mut pushopt = git2::PushOptions::new();
        pushopt.remote_callbacks(git::construct_callbacks(spinner.clone()));
        remote.push(&["refs/heads/main:refs/heads/main"], Some(&mut pushopt))?;
    }
    spinner.success("Successfully initialized confinuum!");

    Ok(())
}
