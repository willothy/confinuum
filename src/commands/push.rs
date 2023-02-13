use anyhow::{Context, Result};
use git2::Repository;
use spinoff::{spinners, Color, Spinner};

use crate::{
    cli::{CreateSharedSpinner, SharedSpinner},
    config::ConfinuumConfig,
    git,
};

pub fn push() -> Result<()> {
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
        let mut pushopt = git2::PushOptions::new();
        pushopt.remote_callbacks(git::construct_callbacks(spinner.clone()));
        spinner.update_text("Pushing changes to remote");
        remote
            .push(&["refs/heads/main:refs/heads/main"], Some(&mut pushopt))
            .with_context(|| format!("Failed to push files to {}", remote.url().unwrap()))?;
        // Scope to ensure that all references to spinner are dropped before we call success
    }
    spinner.success("Changes pushed successfully.");
    Ok(())
}
