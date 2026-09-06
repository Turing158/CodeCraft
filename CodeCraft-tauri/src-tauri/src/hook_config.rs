use std::{env, fs, path::PathBuf};

use serde::{Deserialize, Serialize};

/// Persists the user's hook installation choices separately from the
/// agent-specific hook files. A `true` value means the hook should be kept
/// installed and repaired automatically when CodeCraft starts.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct HookInstallConfig {
    pub claude_code: bool,
    pub codex: bool,
    pub gemini_cli: bool,
    pub open_code: bool,
    pub mimo: bool,
    pub pi: bool,
    pub deep_seek_harness: bool,
    pub z_code: bool,
}

impl Default for HookInstallConfig {
    fn default() -> Self {
        Self {
            claude_code: false,
            codex: false,
            gemini_cli: false,
            open_code: false,
            mimo: false,
            pi: false,
            deep_seek_harness: false,
            z_code: false,
        }
    }
}

fn base_data_dir() -> PathBuf {
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("CodeCraft");
    }
    if let Some(home) = env::var_os("USERPROFILE").or_else(|| env::var_os("HOME")) {
        return PathBuf::from(home).join(".codecraft");
    }
    PathBuf::from(".")
}

fn config_path() -> PathBuf {
    base_data_dir().join("hook-installation.json")
}

pub(crate) fn load() -> HookInstallConfig {
    fs::read(config_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub(crate) fn save(config: &HookInstallConfig) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(config).map_err(|error| error.to_string())?;
    fs::write(path, bytes).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_all_hooks_disabled() {
        let config = HookInstallConfig::default();
        assert!(!config.claude_code);
        assert!(!config.codex);
        assert!(!config.gemini_cli);
        assert!(!config.open_code);
        assert!(!config.mimo);
        assert!(!config.pi);
        assert!(!config.deep_seek_harness);
        assert!(!config.z_code);
    }

    #[test]
    fn accepts_partial_legacy_config() {
        let config: HookInstallConfig = serde_json::from_str(r#"{"codex":true}"#).unwrap();
        assert!(!config.claude_code);
        assert!(config.codex);
        assert!(!config.gemini_cli);
        assert!(!config.open_code);
        assert!(!config.mimo);
        assert!(!config.pi);
        assert!(!config.deep_seek_harness);
        assert!(!config.z_code);
    }
}
