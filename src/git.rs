use anyhow::{anyhow, Context, Result};
use crossterm::{
    cursor::{MoveDown, MoveToColumn},
    style::{self, Print, Stylize},
};
use either::Either;
use git2::{
    Commit, Diff, DiffDelta, DiffFormat, DiffHunk, DiffLine, ObjectType, PackBuilderStage,
    Progress, Repository, Signature,
};
use octocrab::{
    auth::OAuth,
    models::{self},
};
use reqwest::header::ACCEPT;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use spinoff::Spinner;
use std::{cell::RefCell, fs, path::PathBuf, rc::Rc, time::Duration};

use crate::config::{self, ConfinuumConfig};

#[derive(Debug, Serialize, Deserialize)]
pub struct RepoCreateInfo {
    pub name: String,
    pub description: String,
    pub private: bool,
    pub is_template: bool,
    #[serde(flatten)]
    pub opt: Option<RepoCreateInfoOpt>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RepoCreateInfoOpt {
    pub has_downloads: Option<bool>,
    pub homepage: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EmailRes {
    email: String,
    #[allow(dead_code)]
    primary: bool,
    verified: bool,
    visibility: Option<String>,
}

pub struct Github {
    client: octocrab::Octocrab,
}

impl Github {
    pub async fn new() -> anyhow::Result<Self> {
        if Self::is_authenticated() {
            let auth_file = AuthFile::load()?;
            let host = auth_file.auth;
            let auth = OAuth::from(&host);
            return Ok(Self {
                client: octocrab::Octocrab::builder()
                    .oauth(auth)
                    .add_header(ACCEPT, "application/vnd.github+json".to_string())
                    .build()?,
            });
        }

        let auth = Self::authenticate().await?;
        let host = AuthHost::from(&auth);
        let github = Self {
            client: octocrab::Octocrab::builder()
                .oauth(auth)
                .add_header(ACCEPT, "application/vnd.github+json".to_string())
                .build()?,
        };

        // Save the auth token to be reused later
        let auth_file = AuthFile {
            auth: host,
            user: github.get_auth_user().await?,
        };

        auth_file.save()?;

        Ok(github)
    }

    pub async fn get_auth_user(&self) -> anyhow::Result<AuthUser> {
        let res: Vec<EmailRes> = self.client.get("/user/public_emails", None::<&()>).await?;
        let email = res
            .into_iter()
            .find(|e| {
                e.visibility.is_some() && e.visibility.as_ref().unwrap() == "public" && e.verified
            })
            .ok_or_else(|| anyhow!("No primary email found"))?
            .email;
        let user = self.client.current().user().await?;
        Ok(AuthUser {
            name: user.login,
            email,
        })
    }

    pub async fn get_user_signature(&self) -> anyhow::Result<Signature> {
        let user = self.get_auth_user().await?;
        Ok(Signature::now(&user.name, &user.email)?)
    }

    pub fn is_authenticated() -> bool {
        if let Ok(true) = AuthFile::exists() {
            AuthFile::load().is_ok()
        } else {
            false
        }
    }

    async fn authenticate() -> Result<OAuth> {
        let auth_client = octocrab::Octocrab::builder()
            .base_url("https://github.com/")?
            .add_header(ACCEPT, "application/json".to_string())
            .build()?;

        // TODO: Figure out how to get this in without hardcoding it
        let client_id = secrecy::Secret::from("49a3a1366a197af11b86".to_owned());
        let codes = auth_client
            .authenticate_as_device(&client_id, &["public_repo", "repo"])
            .await?;

        println!(
            "Open this link in your browser and enter {}:\n{}",
            codes.user_code, codes.verification_uri
        );
        let mut interval = Duration::from_secs(codes.interval);
        let mut clock = tokio::time::interval(interval);
        let auth = loop {
            clock.tick().await;
            match codes.poll_once(&auth_client, &client_id).await? {
                Either::Left(auth) => break auth,
                Either::Right(cont) => match cont {
                    octocrab::auth::Continue::SlowDown => {
                        // Slow down polling
                        interval += Duration::from_secs(5);
                        clock = tokio::time::interval(interval);
                        clock.tick().await;
                    }
                    octocrab::auth::Continue::AuthorizationPending => {
                        // Keep polling
                    }
                },
            }
        };
        Ok(auth)
    }

    pub async fn create_repo(
        &self,
        repo_info: RepoCreateInfo,
    ) -> anyhow::Result<models::Repository> {
        let new_repo = self
            .client
            .post::<RepoCreateInfo, models::Repository>(
                "https://api.github.com/user/repos",
                Some(&repo_info),
            )
            .await?;
        Ok(new_repo)
    }
}

pub trait RepoExtensions {
    fn find_last_commit(&self) -> anyhow::Result<Commit>;
}

impl RepoExtensions for Repository {
    fn find_last_commit(&self) -> anyhow::Result<Commit> {
        let obj = self.head()?.resolve()?.peel(ObjectType::Commit)?;
        obj.into_commit()
            .map_err(|_| anyhow!("Couldn't find commit"))
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthFile {
    pub user: AuthUser,
    pub auth: AuthHost,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthHost {
    pub token: String,
    pub token_type: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthUser {
    pub name: String,
    pub email: String,
}

impl From<&OAuth> for AuthHost {
    fn from(oauth: &OAuth) -> Self {
        Self {
            token: oauth.access_token.expose_secret().to_owned(),
            token_type: oauth.token_type.to_owned(),
            scopes: oauth.scope.clone(),
        }
    }
}

impl From<&AuthHost> for OAuth {
    fn from(auth_host: &AuthHost) -> Self {
        Self {
            access_token: secrecy::Secret::new(auth_host.token.to_owned()),
            token_type: auth_host.token_type.to_owned(),
            scope: auth_host.scopes.clone(),
        }
    }
}

impl AuthFile {
    pub fn get_path() -> anyhow::Result<std::path::PathBuf> {
        Ok(config::ConfinuumConfig::get_dir()?.join("hosts.toml"))
    }

    pub fn exists() -> anyhow::Result<bool> {
        let path = Self::get_path()?;
        if path.is_dir() {
            return Err(anyhow::anyhow!(
                "Auth file is a directory. Please remove it and try again."
            ));
        }
        Ok(path.exists() && path.is_file())
    }

    pub fn load() -> anyhow::Result<Self> {
        if !Self::exists()? {
            return Err(anyhow::anyhow!(
                "Auth file does not exist. Run `confinuum init` to create one."
            ));
        }
        let path = Self::get_path()?;
        let file = std::fs::read_to_string(&path)
            .with_context(|| format!("Could not read from {}", path.display()))?;
        let auth_file: Self = toml::from_str(&file)?;
        Ok(auth_file)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::get_path()?;
        let file = toml::to_string(&self)?;
        let conf_dir = ConfinuumConfig::get_dir()?;
        if !conf_dir.exists() {
            std::fs::create_dir_all(conf_dir)?;
        }
        fs::write(path, file)?;
        Ok(())
    }
}

/// Remote callbacks
pub fn construct_callbacks<'a>(spinner: Rc<RefCell<Spinner>>) -> git2::RemoteCallbacks<'a> {
    let mut callbacks = git2::RemoteCallbacks::new();
    //let credentials_spinner = spinner.clone();
    callbacks.credentials(
        move |_url: &str, username: Option<&str>, allowed_types: git2::CredentialType| {
            if allowed_types.contains(git2::CredentialType::USERNAME) {
                /* credentials_spinner
                .borrow_mut()
                .update_text(format!("Authenticating with {}", "username".bold())); */
                let username = username.unwrap_or("git");
                return git2::Cred::username(username);
            }

            if allowed_types.contains(git2::CredentialType::SSH_KEY) {
                /* credentials_spinner
                .borrow_mut()
                .update_text(format!("Authenticating with {}", "SSH".bold())); */
                let key = git2::Cred::ssh_key_from_agent(username.unwrap_or("git"));
                match key {
                    Ok(key) => {
                        if key.credtype() == git2::CredentialType::SSH_KEY.bits() {
                            Ok(key)
                        } else {
                            // If there are no identities in the agent, the remote will repeatedly ask for credentials
                            // until the user cancels the operation.
                            // This cancels if the cred is not an SSH key so that we avoid an infinite loop.
                            Err(git2::Error::from_str("No SSH key found"))
                        }
                    }
                    Err(_) => key,
                }
            } else {
                Err(git2::Error::from_str("SSH Auth not supported"))
            }
        },
    );
    //let certificate_spinner = spinner.clone();
    callbacks.certificate_check(move |_cert, _valid| {
        /* certificate_spinner
        .borrow_mut()
        .update_text("Checking certificate"); */
        Ok(git2::CertificateCheckStatus::CertificateOk)
    });
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

pub fn print_diff(diff: &Diff, format: DiffFormat) -> Result<()> {
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
