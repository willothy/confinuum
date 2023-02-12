use std::path::PathBuf;

use crate::{
    cli::{CreateSharedSpinner, SharedSpinner},
    config::ConfinuumConfig,
    git,
};
use anyhow::{anyhow, Context, Result};
use crossterm::style::Stylize;
use git2::{DiffFormat, DiffOptions, Direction, FetchOptions, Repository};
use spinoff::{spinners, Spinner};

// TODO: Update this to use the new config format and check individual entries
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
        spinoff::Color::Blue,
    );

    let (analysis, diff_files) = {
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

        let head = repo.head()?;
        let head_tree = head.peel_to_tree()?;
        let fetch_tree = fetch_head.peel_to_tree()?;
        let mut diff_opt = DiffOptions::default();
        let diff =
            repo.diff_tree_to_tree(Some(&head_tree), Some(&fetch_tree), Some(&mut diff_opt))?;
        let diff_files = git::diff_files(&diff)?;

        if print_diff {
            git::print_diff(&diff, DiffFormat::Patch)?;
        }

        (analysis, diff_files)
    };

    if analysis.0.is_up_to_date() {
        spinner.success("Config is up to date");
    } else {
        spinner.warn(&format!(
            "Config is out of date! Run {} to sync changes.",
            "confinuum update".bold()
        ));
    }

    let (entries, config_updated) = git::diff_entries(&diff_files)?;
    if config_updated {
        println!(
            "\nFound changes in {}{}",
            "config.toml".yellow(),
            if entries.len() > 0 && name.is_none() {
                ""
            } else {
                "\n"
            }
        );
    }
    if let Some(name) = name {
        if entries.contains_key(&name) {
            println!("\nFound remote updates for entry {}\n", name.yellow());
        } else {
            println!(
                "\nNo remote updates found for entry {}\n",
                name.yellow().bold()
            );
        }
    } else {
        if entries.len() > 0 {
            println!(
                "\nFound {} entr{} with remote updates:\n{}\n",
                entries.len().to_string().bold(),
                if entries.len() == 1 { "y" } else { "ies" },
                entries
                    .into_iter()
                    .map(|(name, _)| name.yellow().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }

    crossterm::execute!(std::io::stdout(), crossterm::cursor::Show)?;

    Ok(())
}
