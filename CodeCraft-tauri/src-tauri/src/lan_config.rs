//! Persisted configuration for the LAN web console.
//!
//! The console is off by default and the access token behaves like a password,
//! so this module owns validation, defaults and disk persistence separately
//! from the HTTP server itself.

use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::approval_policy::base_data_dir;

pub(crate) const DEFAULT_PORT: u16 = 8787;
pub(crate) const MIN_PORT: u16 = 1024;
pub(crate) const TOKEN_LENGTH: usize = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum LanBindMode {
    /// Listen on every interface so phones on the same network can connect.
    #[default]
    Lan,
    /// Listen on 127.0.0.1 only, which is useful while debugging locally.
    Loopback,
}

impl LanBindMode {
    pub(crate) fn bind_ip(self) -> [u8; 4] {
        match self {
            Self::Lan => [0, 0, 0, 0],
            Self::Loopback => [127, 0, 0, 1],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct LanServerConfig {
    pub enabled: bool,
    pub port: u16,
    pub bind: LanBindMode,
    pub token: String,
    pub allow_approvals: bool,
    pub audit_remote: bool,
}

impl Default for LanServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_PORT,
            bind: LanBindMode::default(),
            token: generate_token(),
            allow_approvals: false,
            audit_remote: true,
        }
    }
}

/// Builds a 32-character lowercase hex token from two UUID v4 values so the
/// crate already in the dependency tree provides the randomness.
pub(crate) fn generate_token() -> String {
    let mut token = String::with_capacity(TOKEN_LENGTH);
    while token.len() < TOKEN_LENGTH {
        token.push_str(&uuid::Uuid::new_v4().simple().to_string());
    }
    token.truncate(TOKEN_LENGTH);
    token
}

/// Rejects privileged ports so enabling the console never needs elevation.
pub(crate) fn validate_port(port: u16) -> Result<u16, String> {
    if port < MIN_PORT {
        return Err(format!("端口需要在 {MIN_PORT}-65535 之间"));
    }
    Ok(port)
}

fn normalize_token(token: &str) -> Option<String> {
    let trimmed = token.trim();
    if trimmed.len() < 16
        || !trimmed
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(trimmed.to_string())
}

/// Repairs values that a hand-edited config file could have broken.
pub(crate) fn normalize_config(mut config: LanServerConfig) -> LanServerConfig {
    if validate_port(config.port).is_err() {
        config.port = DEFAULT_PORT;
    }
    config.token = normalize_token(&config.token).unwrap_or_else(generate_token);
    config
}

fn config_path() -> PathBuf {
    base_data_dir().join("lan-server.json")
}

pub(crate) fn load_config() -> LanServerConfig {
    let loaded = fs::read(config_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<LanServerConfig>(&bytes).ok());
    match loaded {
        Some(config) => normalize_config(config),
        None => {
            let config = LanServerConfig::default();
            let _ = save_config(&config);
            config
        }
    }
}

pub(crate) fn save_config(config: &LanServerConfig) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(config).map_err(|error| error.to_string())?;
    fs::write(path, bytes).map_err(|error| error.to_string())
}

/// Compares tokens without leaking their matching prefix length through timing.
pub(crate) fn tokens_match(expected: &str, provided: &str) -> bool {
    let expected = expected.as_bytes();
    let provided = provided.as_bytes();
    if expected.is_empty() || expected.len() != provided.len() {
        return false;
    }
    let mut difference = 0u8;
    for index in 0..expected.len() {
        difference |= expected[index] ^ provided[index];
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_console_starts_disabled_with_remote_approvals_off() {
        let config = LanServerConfig::default();

        assert!(!config.enabled);
        assert!(!config.allow_approvals);
        assert!(config.audit_remote);
        assert_eq!(config.port, DEFAULT_PORT);
        assert_eq!(config.token.len(), TOKEN_LENGTH);
    }

    #[test]
    fn generated_tokens_are_unique_hex_strings() {
        let first = generate_token();
        let second = generate_token();

        assert_ne!(first, second);
        assert!(first.chars().all(|character| character.is_ascii_hexdigit()));
    }

    #[test]
    fn privileged_ports_are_rejected() {
        assert!(validate_port(80).is_err());
        assert!(validate_port(1023).is_err());
        assert_eq!(validate_port(8787), Ok(8787));
    }

    #[test]
    fn a_hand_edited_config_is_repaired_instead_of_trusted() {
        let repaired = normalize_config(LanServerConfig {
            port: 42,
            token: "short".to_string(),
            ..LanServerConfig::default()
        });

        assert_eq!(repaired.port, DEFAULT_PORT);
        assert_eq!(repaired.token.len(), TOKEN_LENGTH);
    }

    #[test]
    fn token_comparison_requires_an_exact_match() {
        assert!(tokens_match("abc123", "abc123"));
        assert!(!tokens_match("abc123", "abc124"));
        assert!(!tokens_match("abc123", "abc1234"));
        assert!(!tokens_match("", ""));
    }

    #[test]
    fn bind_modes_map_to_the_expected_interfaces() {
        assert_eq!(LanBindMode::Lan.bind_ip(), [0, 0, 0, 0]);
        assert_eq!(LanBindMode::Loopback.bind_ip(), [127, 0, 0, 1]);
    }
}
