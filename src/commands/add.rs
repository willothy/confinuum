use crate::{
    cli::{CreateSharedSpinner, SharedSpinner},
    config::ConfinuumConfig,
    git::{self, Github, RepoExtensions},
};
use anyhow::{anyhow, Context, Result};
use git2::{FetchOptions, IndexAddOption, Repository};
use spinoff::{spinners, Color, Spinner};
use std::{collections::HashSet, path::PathBuf};

/// Add files to an existing config entry
pub async fn add(name: String, files: Vec<PathBuf>, push: bool, github: &Github) -> Result<()> {
    let config_dir = ConfinuumConfig::get_dir().context("Failed to fetch config dir")?;
    let repo = Repository::open(&config_dir)
        .with_context(|| format!("Could not open repository in {}", config_dir.display()))?;
    let mut remote = repo.find_remote("origin")?;
    let spinner = Spinner::new_shared(
        spinners::Dots9,
        "Connecting to remote 'origin'",
        Color::Blue,
    );
    {
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
            spinner.update_text("No changes found on remote, continuing");
        } else {
            spinner.fail("Changes found on remote");
            return Err(anyhow!(
                "Changes found on remote. Please pull them before adding files."
            ));
        }

        let mut config = ConfinuumConfig::load()?;
        if !config.entries.contains_key(&name) {
            return Err(anyhow!(
                "Entry named {} does not exist! Use the `new` subcommand to create it.",
                name
            ));
        }

        let entry = config.entries.get_mut(&name).unwrap();
        let mut result_files = HashSet::new();
        ConfinuumConfig::add_files_recursive(entry, files, None, &mut Some(&mut result_files))
            .context("Failed to add files to config")?;
        config.save().context("Failed to save config file")?;

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
        let parent_commit = repo
            .find_last_commit()
            .context("Failed to retrieve last commit")?;
        let sig = git::get_git_user_signature().unwrap_or(github.get_user_signature().await?);
        let tree = repo
            .find_tree(oid)
            .context("Failed to find new commit tree")?;
        let message = format!(
            "Added {} files to `{}`\n\nNew files:\n{}",
            result_files.len(),
            name,
            result_files
                .iter()
                .map(|f| f.display().to_string())
                .collect::<Vec<_>>()
                .join("\n")
        );

        repo.commit(Some("HEAD"), &sig, &sig, &message, &tree, &[&parent_commit])
            .context("Failed to commit files")?;

        crate::util::deploy(Some(&name))?;
    }

    spinner.success("Files added successfully");

    if push {
        let spinner = Spinner::new_shared(
            spinners::Dots9,
            "Connecting to remote 'origin'",
            Color::Blue,
        );
        {
            let mut pushopt = git2::PushOptions::new();
            pushopt.remote_callbacks(git::construct_callbacks(spinner.clone()));
            spinner.update_text("Pushing changes to remote");
            remote
                .push(&["refs/heads/main:refs/heads/main"], Some(&mut pushopt))
                .with_context(|| format!("Failed to push files to {}", remote.url().unwrap()))?;
            // Scope to ensure that all references to spinner are dropped before we call success
        }
        spinner.success("Changes pushed successfully.");
    }

    Ok(())
}
