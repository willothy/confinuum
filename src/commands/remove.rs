use std::{fs, path::PathBuf};

use anyhow::{anyhow, Context, Result};
use crossterm::style::Stylize;
use git2::{FetchOptions, IndexAddOption, Repository};
use spinoff::{spinners, Color, Spinner};

use crate::{
    cli::{CreateSharedSpinner, SharedSpinner},
    config::{ConfinuumConfig, SignatureSource},
    git::{self, RepoExtensions},
    github::Github,
};

pub(crate) async fn remove(
    name: String,
    mut files: Vec<PathBuf>,
    no_confirm: bool,
    no_replace_files: bool,
    push: bool,
    github: &Github,
) -> Result<()> {
    // Ensure entry exists
    let config_dir = ConfinuumConfig::get_dir().context("Cannot get config dir")?;
    let mut config = ConfinuumConfig::load().context("Cannot load config file")?;
    if !config.entries.contains_key(&name) {
        return Err(anyhow!("No entry named {} found", name.red().bold()));
    }

    // Ensure all files exist
    files.iter_mut().try_for_each(|f| -> Result<()> {
        *f = f
            .canonicalize()
            .context(format!("Could not canonicalize {}", f.display()))?;
        Ok(())
    })?;
    for file in &files {
        if !file.exists() {
            return Err(anyhow!(
                "File {} does not exist",
                file.display().to_string().red().bold()
            ));
        }
    }

    let entry = config
        .entries
        .get_mut(&name)
        .ok_or_else(|| anyhow!("No entry named {} found", name))?;

    // Ensure all files are in the entry
    for file in &files {
        let file = file.strip_prefix(&config_dir.join(&name)).context(format!(
            "cannot strip prefix {} from {}",
            config_dir.join(&name).display(),
            file.display()
        ))?;
        if !entry.files.contains(file) {
            return Err(anyhow!(
                "File {} does not exist in entry {}",
                file.display().to_string().red().bold(),
                name.yellow().bold()
            ));
        }
    }

    // Ensure there aren't changes on remote
    let repo = Repository::open(&config_dir)?;
    let mut remote = repo.find_remote("origin")?;
    let spinner = Spinner::new_shared(
        spinners::Dots9,
        "Connecting to remote 'origin'",
        Color::Blue,
    );
    {
        // Scope to ensure that all references to spinner are dropped before we call success
        spinner.update_text("Checking for changes on remote");
        let mut fetch_opt = FetchOptions::new();
        fetch_opt.update_fetchhead(true);
        fetch_opt.remote_callbacks(git::construct_callbacks(spinner.clone()));
        remote
            .fetch(&["main"], Some(&mut fetch_opt), None)
            .context("Failed to fetch from remote 'origin'")?;
        let fetch_head = repo.find_reference("FETCH_HEAD")?;
        let fetch_commit = repo.reference_to_annotated_commit(&fetch_head)?;
        // Check if up to date
        let analysis = repo.merge_analysis(&[&fetch_commit])?;
        remote.disconnect()?;
        if !analysis.0.is_up_to_date() {
            spinner.fail("Changes found on remote");
            return Err(anyhow!(
                "Changes found on remote. Please pull them before deleting files."
            ));
        }
    }
    spinner.clear();

    let confirm = no_confirm || {
        let selection = dialoguer::Select::new()
            .with_prompt(format!(
                "Are you sure you want to delete {} files from {}?",
                files.len(),
                name.clone().yellow().bold()
            ))
            .items(&["Yes", "No"])
            .default(1)
            .interact_opt()
            .context("Failed to interact with user, cancelling.")?;
        if selection != Some(0) {
            false // User selected no or cancelled
        } else {
            true
        }
    };
    if !confirm {
        return Ok(());
    }

    let spinner = Spinner::new_shared(
        spinners::Dots9,
        format!(
            "Confirmed removal of {} files from {}, continuing",
            files.len(),
            &name
        ),
        Color::Blue,
    );

    super::undeploy(Some(&name))?; // Undeploy entry if it's deployed

    {
        // Remove files from entry, and move them to their original location (unless no)
        let mut removed_files = Vec::new();
        for file in &files {
            let file = file.strip_prefix(&config_dir.join(&name)).context(format!(
                "cannot strip prefix {} from {}",
                config_dir.join(&name).display(),
                file.display()
            ))?;
            spinner.update_text(format!("Removing {}", file.display()));
            entry.files.remove(file);
            removed_files.push(file.to_path_buf());
            let source_path = config_dir.join(&name).join(&file);
            let target_path = entry.target_dir.as_ref().unwrap().join(&file);
            if !no_replace_files {
                fs::copy(&source_path, &target_path).with_context(|| {
                    format!(
                        "Cannot copy {} to {}",
                        source_path.display(),
                        target_path.display()
                    )
                })?;
            }
            fs::remove_file(&source_path)
                .with_context(|| format!("Cannot remove {}", source_path.display()))?;
        }

        spinner.update_text(format!("Saving config file"));

        config.save()?;

        spinner.update_text(format!("Committing changes"));
        let mut index = repo.index()?;
        let mut imp = |path: &std::path::Path, _data: &[u8]| {
            if path.starts_with(".git") {
                return 1; // skip .git/
            }
            return 0;
        };
        index
            .add_all(["*"], IndexAddOption::DEFAULT, Some(&mut imp))
            .context("Could not add files")?;
        let oid = index.write_tree().context("Failed to write tree")?;
        // Get the last commit
        let parent_commit = repo
            .find_last_commit()
            .context("Failed to retrieve last commit")?;
        // Await the user signature from the GitHub API
        let sig = match &config.confinuum.signature_source {
            SignatureSource::Github => github
                .get_user_signature()
                .await
                .context("Could not fetch user signature from github")?,
            SignatureSource::GitConfig => {
                // allows users to set values in config if they don't exist
                git::gitconfig::get_user_sig()?
            }
        };
        let tree = repo
            .find_tree(oid)
            .context("Failed to find new commit tree")?;
        let message = format!(
            "Deleted {} files from `{}`\n\nDeleted files:\n{}",
            files.len(),
            name,
            removed_files
                .iter()
                .map(|f| f.display().to_string())
                .collect::<Vec<_>>()
                .join("\n")
        );

        repo.commit(Some("HEAD"), &sig, &sig, &message, &tree, &[&parent_commit])
            .context("Failed to commit files")?;

        if push {
            // Push the changes
            spinner.update_text("Pushing changes to remote");
            let mut pushopt = git2::PushOptions::new();
            pushopt.remote_callbacks(git::construct_callbacks(spinner.clone()));
            remote
                .push(&["refs/heads/main:refs/heads/main"], Some(&mut pushopt))
                .with_context(|| format!("Failed to push files to {}", remote.url().unwrap()))?;
        }
    }
    super::deploy(Some(&name))?; // Deploy entry
    spinner.success(&format!(
        "Successfully removed {} files from {}",
        files.len(),
        &name
    ));

    Ok(())
}
