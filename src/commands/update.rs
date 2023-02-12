use crate::{
    cli::{CreateSharedSpinner, SharedSpinner},
    config::ConfinuumConfig,
    git, util,
};
use anyhow::{anyhow, Context, Result};
use crossterm::style::Stylize;
use git2::{DiffOptions, Direction, FetchOptions, Repository};
use spinoff::{spinners, Spinner};

pub fn update() -> Result<()> {
    // TODO: Check for local unstaged changes
    util::undeploy(None::<&str>)?;

    let config_dir = ConfinuumConfig::get_dir()?;
    if !config_dir.exists() {
        return Err(anyhow!("Config directory does not exist"));
    }
    let repo =
        Repository::open(config_dir).context("Failed to open config directory as a git repo")?;
    let mut remote = repo
        .find_remote("origin")
        .context("Failed to find remote named 'origin'")?;
    crossterm::execute!(std::io::stdout(), crossterm::cursor::Hide)?;
    let spinner = Spinner::new_shared(
        spinners::Dots9,
        "Connecting to remote 'origin'",
        spinoff::Color::Blue,
    );

    let (analysis, diff_files, fetch_commit, head_commit) = {
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

        let head = repo.head()?;
        let head_commit = repo.reference_to_annotated_commit(&head)?;
        let head_tree = head.peel_to_tree()?;
        let fetch_tree = fetch_head.peel_to_tree()?;
        let mut diff_opt = DiffOptions::default();
        let diff =
            repo.diff_tree_to_tree(Some(&head_tree), Some(&fetch_tree), Some(&mut diff_opt))?;
        let diff_files = git::diff_files(&diff)?;

        (analysis, diff_files, fetch_commit, head_commit)
    };

    let (diff_entries, config_updated) = git::diff_entries(&diff_files)?;

    if analysis.0.is_up_to_date() {
        spinner.success("Already up to date");
    } else if analysis.0.is_unborn() {
        spinner.success("Already up to date");
    } else if analysis.0.is_none() {
        spinner.success("Already up to date");
    } else if analysis.0.is_fast_forward() {
        spinner.update_text("Applying changes");
        let refname = "refs/heads/main";
        let mut reference = repo.find_reference(refname)?;
        reference.set_target(fetch_commit.id(), "Fast-Forward")?;
        repo.set_head(refname)?;
        repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;
        spinner.success("Changes pulled succesfully");
    } else if analysis.0.is_normal() {
        spinner.update_text("Merging changes");
        let local_tree = repo.find_commit(head_commit.id())?.tree()?;
        let remote_tree = repo.find_commit(fetch_commit.id())?.tree()?;
        let ancestor = repo
            .find_commit(repo.merge_base(head_commit.id(), fetch_commit.id())?)?
            .tree()?;
        let mut idx = repo.merge_trees(&ancestor, &local_tree, &remote_tree, None)?;

        if idx.has_conflicts() {
            repo.checkout_index(Some(&mut idx), None)?;
            spinner.fail("Merge conflicts detected, aborting");
            return Ok(());
        }
        let result_tree = repo.find_tree(idx.write_tree_to(&repo)?)?;
        // now create the merge commit
        let msg = format!(
            "Merge {} into {}\n\nFiles changed:\n{}",
            fetch_commit.id(),
            head_commit.id(),
            {
                let mut s = String::new();
                if config_updated {
                    s.push_str("config.toml\n");
                }
                for (entry, changed_files) in diff_entries {
                    s.push_str(&format!("{}:\n", entry.bold().yellow()));
                    for file in changed_files {
                        s.push_str(&format!("    {}\n", file.display()));
                    }
                }
                s
            }
        );
        let sig = repo.signature()?;
        let local_commit = repo.find_commit(head_commit.id())?;
        let remote_commit = repo.find_commit(fetch_commit.id())?;

        let _merge_commit = repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            &msg,
            &result_tree,
            &[&local_commit, &remote_commit],
        )?;

        repo.checkout_head(None)?;

        spinner.update_text("Pushing merged changes");

        let mut push_opt = git2::PushOptions::default();
        push_opt.remote_callbacks(git::construct_callbacks(spinner.clone()));
        remote
            .push(&["refs/heads/main:refs/heads/main"], Some(&mut push_opt))
            .with_context(|| format!("Failed to push files to {}", remote.url().unwrap()))?;

        spinner.success("Changes merged succesfully");
    } else {
        spinner.fail("Unknown merge analysis, aborting");
        return Ok(());
    }

    util::deploy(None::<&str>)?;

    Ok(())
}
