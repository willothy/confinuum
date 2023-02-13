//! Git-related functionality for confinuum

use anyhow::{anyhow, Context, Result};
use crossterm::{
    cursor::MoveToColumn,
    style::{self, Print, Stylize},
};
use dialoguer::theme::ColorfulTheme;

use email_address::EmailAddress;
use git2::{
    Commit, Config, Diff, DiffDelta, DiffFormat, DiffHunk, DiffLine, ObjectType, PackBuilderStage,
    Progress, Repository, Signature,
};

use spinoff::Spinner;

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
};

use crate::config::ConfinuumConfig;

pub(crate) trait RepoExtensions {
    fn find_last_commit(&self) -> anyhow::Result<Commit>;
}

impl RepoExtensions for Repository {
    fn find_last_commit(&self) -> anyhow::Result<Commit> {
        let obj = self.head()?.resolve()?.peel(ObjectType::Commit)?;
        obj.into_commit()
            .map_err(|_| anyhow!("Couldn't find commit"))
    }
}

fn find_ssh_key() -> anyhow::Result<PathBuf> {
    let ssh_dir =
        PathBuf::from(std::env::var("HOME").context("Could not find home directory")?).join(".ssh");

    let key = vec!["id_ed25519", "id_rsa", "id_ecdsa", "id_dsa"]
        .into_iter()
        .map(|key| ssh_dir.join(key))
        .find(|key| key.exists())
        .ok_or_else(|| anyhow!("No SSH key found"))?;

    Ok(key)
}

/// Remote callbacks
pub(crate) fn construct_callbacks<'a>(spinner: Rc<RefCell<Spinner>>) -> git2::RemoteCallbacks<'a> {
    let mut callbacks = git2::RemoteCallbacks::new();
    callbacks.credentials(
        move |url: &str, username: Option<&str>, allowed_types: git2::CredentialType| {
            if allowed_types.contains(git2::CredentialType::USERNAME) {
                let username = username.unwrap_or("git");
                return git2::Cred::username(username);
            }

            if allowed_types.contains(git2::CredentialType::SSH_KEY)
                || allowed_types.contains(git2::CredentialType::DEFAULT)
            {
                let key_path = find_ssh_key()
                    .map_err(|_| git2::Error::from_str("Could not find SSH key in ~/.ssh"))?;
                return git2::Cred::ssh_key(
                    username.unwrap_or("git"),
                    None,
                    key_path.as_path(),
                    None,
                );
            }

            if allowed_types.contains(git2::CredentialType::SSH_MEMORY) {
                let key_path = find_ssh_key()
                    .map_err(|_| git2::Error::from_str("Could not find SSH key in ~/.ssh"))?;
                let key = std::fs::read_to_string(key_path)
                    .map_err(|_| git2::Error::from_str("Could not read SSH key"))?;
                return git2::Cred::ssh_key_from_memory(
                    username.unwrap_or("git"),
                    None,
                    &key,
                    None,
                );
            }

            if allowed_types.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
                let config = git2::Config::open_default()?;
                if let Ok(cred) = git2::Cred::credential_helper(&config, url, username) {
                    return Ok(cred);
                } else {
                    let username = username.unwrap_or("git");
                    let password =
                        rpassword::prompt_password(format!("Password for '{}': ", username))
                            .map_err(|_| git2::Error::from_str("Could not prompt for password"))?;
                    return git2::Cred::userpass_plaintext(username, &password);
                }
            }

            return Err(git2::Error::from_str("SSH Auth type not supported"));
        },
    );
    callbacks
        .certificate_check(move |_cert, _valid| Ok(git2::CertificateCheckStatus::CertificateOk));
    let transfer_spinner = spinner.clone();
    callbacks.transfer_progress(move |stats: Progress| {
        let received_objects = stats.received_objects();
        let total_objects = stats.total_objects();

        let recv_done = received_objects == total_objects;
        transfer_spinner.borrow_mut().update_text(format!(
            "Receiving objects: {}% ({}/{}){}",
            (received_objects as f64 / total_objects as f64 * 100.) as usize,
            received_objects,
            total_objects,
            recv_done.then_some(", done.").unwrap_or_default()
        ));
        true
    });
    let push_update_spinner = spinner.clone();
    callbacks.push_update_reference(move |refname: &str, status: Option<&str>| {
        if let Some(status) = status {
            push_update_spinner
                .clone()
                .borrow_mut()
                .update_text(format!("Updated {}: {}", refname, status));
        }
        Ok(())
    });
    let push_transfer_spinner = spinner.clone();
    callbacks.push_transfer_progress(move |progress: usize, total: usize, bytes: usize| {
        push_transfer_spinner
            .clone()
            .borrow_mut()
            .update_text(format!(
                "Writing objects: {} / {} ({} bytes)",
                progress, total, bytes
            ));
    });
    let tips_spinner = spinner.clone();
    callbacks.update_tips(move |refname: &str, old: git2::Oid, new: git2::Oid| {
        tips_spinner.clone().borrow_mut().update_text(format!(
            "{}: {} -> {}",
            refname,
            &old.to_string()[0..7],
            &new.to_string()[0..7]
        ));
        true
    });
    let sideband_spinner = spinner.clone();
    callbacks.sideband_progress(move |data: &[u8]| {
        let message = String::from_utf8(data.to_vec()).ok();
        if let Some(message) = message {
            sideband_spinner
                .clone()
                .borrow_mut()
                .update_text(format!("remote: {}", message.trim_end()));
        }
        true
    });
    let pack_spinner = spinner.clone();
    callbacks.pack_progress(
        move |stage: PackBuilderStage, current: usize, total: usize| {
            let done = if current >= total { ", done." } else { "." };
            match stage {
                PackBuilderStage::AddingObjects => pack_spinner
                    .clone()
                    .borrow_mut()
                    .update_text(format!("Adding objects: {}{}", current, done)),
                PackBuilderStage::Deltafication => {
                    pack_spinner.clone().borrow_mut().update_text(format!(
                        "Resolving deltas: ({}%) {} / {}{}",
                        current as f64 / total as f64,
                        current,
                        total,
                        done
                    ));
                }
            }
        },
    );
    callbacks
}

pub(crate) fn print_diff(diff: &Diff, format: DiffFormat) -> Result<()> {
    let mut stdout = std::io::stdout().lock();

    crossterm::queue!(stdout, MoveToColumn(0) /* Print("\n") */)?;
    diff.print(
        format,
        |_delta: DiffDelta, _hunk: Option<DiffHunk>, line: DiffLine| -> bool {
            use crossterm::style::Color::*;
            let mut style = style::ContentStyle::new();
            let mut origin = "";
            match line.origin_value() {
                git2::DiffLineType::Addition => {
                    style.foreground_color = Some(Green);
                    origin = "+";
                }
                git2::DiffLineType::Deletion => {
                    style.foreground_color = Some(Red);
                    origin = "-";
                }
                git2::DiffLineType::FileHeader => {
                    style.foreground_color = Some(Reset);
                    style.attributes.set(style::Attribute::Bold);
                }
                git2::DiffLineType::HunkHeader => {
                    style.foreground_color = Some(Blue);
                }
                git2::DiffLineType::Binary => {
                    style.foreground_color = Some(Reset);
                    style.attributes.set(style::Attribute::Bold);
                }
                _ => {}
            }

            crossterm::queue!(
                stdout,
                Print(style.apply(format!(
                    "{}{}{}\n",
                    origin,
                    String::from_utf8(line.content().to_vec())
                        .unwrap_or_default()
                        .trim_end(),
                    if line.origin_value() == git2::DiffLineType::HunkHeader {
                        "\n"
                    } else {
                        ""
                    }
                ))),
            )
            .ok();
            true
        },
    )?;

    crossterm::queue!(stdout, Print("\n"))?;
    std::io::Write::flush(&mut stdout)?;
    Ok(())
}

pub(crate) fn diff_files(diff: &Diff) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for delta in diff.deltas() {
        if let Some(file) = delta.new_file().path().map(|p| p.to_path_buf()) {
            files.push(file);
        }
    }
    Ok(files)
}

pub(crate) fn diff_entries(
    files: &Vec<PathBuf>,
) -> Result<(HashMap<String, HashSet<PathBuf>>, bool)> {
    let mut entries = HashMap::new();
    let config = ConfinuumConfig::load()?;
    let mut config_updated = false;
    for file in files {
        let components = file.components();
        if components.count() == 1 {
            // File is in root of config directory
            if file.components().next().unwrap().as_os_str() == "config.toml" {
                config_updated = true;
            }

            continue;
        }
        let entry = file
            .components()
            .next()
            .unwrap()
            .as_os_str()
            .to_string_lossy()
            .to_string();
        if config.entries.contains_key(&entry) {
            if entries.contains_key(&entry) {
                let entry_files: &mut HashSet<PathBuf> = entries.get_mut(&entry).unwrap();
                entry_files.insert(file.to_path_buf());
            } else {
                entries.insert(entry, HashSet::from_iter(vec![file.to_path_buf()]));
            }
        } else {
            return Err(anyhow!(
                "Found file that does not belong to any entry: {}",
                file.display()
            ));
        }
    }
    Ok((entries, config_updated))
}

pub(crate) mod gitconfig {
    use super::*;
    pub(crate) fn git_config() -> Result<Config> {
        let path = git2::Config::find_global().context("Failed to find global git config")?;
        git2::Config::open(&path).context("Failed to open global (user-level) git config")
    }

    pub(crate) fn get_user_name() -> Result<String> {
        let config = git_config()?;
        config
            .get_string("user.name")
            .context("Failed to get user.name from git config")
    }

    pub(crate) fn get_user_email() -> Result<EmailAddress> {
        let config = git_config()?;
        config
            .get_string("user.email")
            .context("Failed to get user.email from git config")?
            .parse()
            .context("Could not parse email address")
    }

    /// Retrieve git config user.name and user.email and return a git2::Signature
    /// Throw an error if either of the values are not set
    pub(crate) fn get_user_sig() -> Result<Signature<'static>> {
        let name = get_user_name()?;
        let email = get_user_email()?;
        Ok(Signature::now(&name, &email.to_string())?)
    }

    /// Retrieve git config user.name and user.email and return a git2::Signature
    /// Prompt the user to set the values if they are not set
    pub(crate) fn get_user_sig_with_prompt() -> Result<Signature<'static>> {
        let username = if let Ok(username) = get_user_name() {
            username
        } else {
            let username: String = dialoguer::Input::with_theme(&ColorfulTheme::default())
                .with_prompt(format!(
                    "It looks like you haven't set {} in your git config. Enter the name you want to use for git commits",
                    "user.name".bold()
                ))
                .interact()?;
            let add_to_gitconfig = dialoguer::Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt("Do you want to add this to your git config?")
                .interact()?;
            if add_to_gitconfig {
                let mut config = git_config()?;
                config.set_str("user.name", &username)?;
            }
            username
        };

        let email = match get_user_email() {
            Ok(email) => email,
            Err(e) => {
                let mut err = e.to_string();
                err.truncate(30);
                let email: EmailAddress = dialoguer::Input::with_theme(&ColorfulTheme::default())
                    .with_prompt(format!(
                        "Could not find {} in git config ({}). Enter the email you want to use for git commits",
                        "user.email".bold(),
                        err
                    ))
                    .interact()?;

                let add_to_gitconfig = dialoguer::Confirm::with_theme(&ColorfulTheme::default())
                    .with_prompt("Do you want to add this to your git config?")
                    .interact()?;
                if add_to_gitconfig {
                    let mut config = git_config()?;
                    config.set_str("user.email", &email.to_string())?;
                }

                email
            }
        };

        Signature::now(&username, &email.to_string()).context(format!(
            "Failed to create git signature from {} and {}",
            username, email
        ))
    }
}
