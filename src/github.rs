use crate::config::{self, ConfinuumConfig};
use anyhow::{anyhow, Context, Result};
use either::Either;
use git2::Signature;
use octocrab::{auth::OAuth, models};
use reqwest::header::ACCEPT;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use std::{fs, time::Duration};

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
