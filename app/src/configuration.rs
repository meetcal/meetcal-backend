use config::{Config, Environment, File};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
pub struct Settings {
    pub database: DatabaseSettings,
    pub application_host: String,
    pub application_port: u16,
    #[serde(default)]
    pub auth: AuthSettings,
}

/// Clerk JWT verification settings.
///
/// `jwt_verification_enabled` is the master switch and DEFAULTS TO TRUE (fail
/// closed): a config that omits it, or sets it false without the explicit
/// `APP_ALLOW_UNVERIFIED_JWT=1` escape hatch, makes startup PANIC rather than
/// silently run unauthenticated. The test harness sets that env var.
#[derive(Deserialize, Clone)]
pub struct AuthSettings {
    #[serde(default = "default_true")]
    pub jwt_verification_enabled: bool,
    /// Clerk JWKS endpoint, e.g. `https://<subdomain>.clerk.accounts.dev/.well-known/jwks.json`.
    #[serde(default)]
    pub jwks_url: String,
    /// Expected `iss` claim, e.g. `https://<subdomain>.clerk.accounts.dev`.
    #[serde(default)]
    pub issuer: String,
}

fn default_true() -> bool {
    true
}

impl Default for AuthSettings {
    fn default() -> Self {
        Self {
            jwt_verification_enabled: true,
            jwks_url: String::new(),
            issuer: String::new(),
        }
    }
}

#[derive(Deserialize)]
pub struct DatabaseSettings {
    pub username: String,
    #[serde(default)]
    pub password: String,
    pub port: u16,
    pub host: String,
    pub database_name: String,
}

pub fn get_configuration() -> Result<Settings, config::ConfigError> {
    let config_dir = std::env::var("APP_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"));

    Config::builder()
        .add_source(File::from(config_dir.join("configuration.yaml")))
        .add_source(File::from(config_dir.join("configuration.local.yaml")).required(false))
        .add_source(
            Environment::with_prefix("APP")
                .prefix_separator("_")
                .separator("__"),
        )
        .build()?
        .try_deserialize()
}

impl DatabaseSettings {
    pub fn password(&self) -> Result<String, config::ConfigError> {
        if !self.password.is_empty() {
            return Ok(self.password.clone());
        }
        std::env::var("APP_DATABASE__PASSWORD").map_err(|_| {
            config::ConfigError::Message(
                "database password missing: set APP_DATABASE__PASSWORD in meetcal-backend/.env"
                    .into(),
            )
        })
    }

    pub fn connection_string(&self) -> Result<String, config::ConfigError> {
        let password = self.password()?;
        Ok(format!(
            "postgres://{}:{}@{}:{}/{}",
            percent_encode(&self.username),
            percent_encode(&password),
            self.host,
            self.port,
            percent_encode(&self.database_name)
        ))
    }

    pub fn connection_string_without_db(&self) -> Result<String, config::ConfigError> {
        let password = self.password()?;
        Ok(format!(
            "postgres://{}:{}@{}:{}",
            percent_encode(&self.username),
            percent_encode(&password),
            self.host,
            self.port
        ))
    }
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::new();

    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            _ => {
                encoded.push_str(&format!("%{byte:02X}"));
            }
        }
    }

    encoded
}
