use crate::{
    cli::{CreateSharedSpinner, SharedSpinner},
    config::{ConfinuumConfig, SignatureSource},
    git::{self, Github, RepoExtensions},
};
use anyhow::{anyhow, Context, Result};
use git2::{FetchOptions, IndexAddOption, Repository};
use spinoff::{spinners, Color, Spinner};

/// Remove a config entry (files will be restored to their original locations unless no_replace_files is set)
pub async fn delete(
    name: String,
    no_confirm: bool,
    no_replace_files: bool,
    push: bool,
    github: &Github,
) -> Result<()> {
    // Load config file
    let mut config = ConfinuumConfig::load()?;
    let config_dir = ConfinuumConfig::get_dir()?;

    // Ensure that the entry exists
    if !config.entries.contains_key(&name) {
        return Err(anyhow!("No entry named {}", name));
    }

    // Ensure that there aren't unfetched changes on the remote
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
                "Are you sure you want to delete the entry {}?",
                name
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

    // Perform the actual deletion
    let spinner = Spinner::new_shared(
        spinners::Dots9,
        "Confirmed deletion, continuing",
        Color::Blue,
    );
    {
        // Scope to ensure that all references to spinner are dropped before we call success
        let entry = config.entries.get(&name).unwrap();
        if no_replace_files {
            // Delete deployed symlinks
            spinner.update_text("Skipping file restoration, deleting symlinks");
            for file in entry.files.iter() {
                let target_path = entry.target_dir.as_ref().ok_or(anyhow!(
                "Entry {} does not have a target directory, cannot restore files. Cancelling deletion.",
                name
            ))?.join(file);
                std::fs::remove_file(&target_path)
                    .with_context(|| format!("Cannot remove {}", target_path.display()))?;
            }
        } else {
            // Restore files to their original locations, and delete symlinks
            spinner.update_text("Restoring files to original locations");
            for file in entry.files.iter() {
                let target_path = entry.target_dir.as_ref().ok_or(anyhow!(
                "Entry {} does not have a target directory, cannot restore files. Cancelling deletion.",
                name
            ))?.join(file);
                let repo_path = config_dir.join(&name).join(&file);
                if target_path.exists() {
                    std::fs::remove_file(&target_path)
                        .with_context(|| format!("Cannot remove {}", target_path.display()))?;
                }
                std::fs::copy(&repo_path, &target_path).with_context(|| {
                    format!(
                        "Cannot copy {} to {}",
                        repo_path.display(),
                        target_path.display()
                    )
                })?;
            }
        }
        spinner.update_text("Deleting files from repository");
        // Delete the entry's folder in the repo
        std::fs::remove_dir_all(config_dir.join(&name)).with_context(|| {
            format!(
                "Cannot recursively remove dir {}",
                config_dir.join(&name).display()
            )
        })?;
        // Safe to unwrap because we checked that the entry exists earlier
        // Save this to add deleted files to commit message
        let removed_entry = config.entries.remove(&name).unwrap();

        // Write the new config file
        config.save()?;

        spinner.update_text("Committing changes");

        // Commit the changes
        let mut index = repo.index()?;
        let mut imp = |path: &std::path::Path, _data: &[u8]| {
            if path.starts_with(".git") {
                return 1; // skip .git/
            }
            return 0;
        };
        // Add all files to the index
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
            "Deleted entry `{}`\n\nDeleted files:\n{}",
            name,
            removed_entry
                .files
                .iter()
                .map(|f| f.display().to_string())
                .collect::<Vec<_>>()
                .join("\n")
        );

        // Make the commit
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
    // All done!
    spinner.success("Successfully deleted entry");

    Ok(())
}
