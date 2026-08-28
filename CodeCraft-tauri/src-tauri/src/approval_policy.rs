use std::{env, fs, path::PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ApprovalMode {
    #[default]
    Manual,
    Risk,
    Automatic,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct ApprovalSettings {
    pub mode: ApprovalMode,
    /// Keeps the native panel hidden while approval hooks and the LAN service
    /// continue running with the selected policy.
    pub minimal_mode: bool,
}

pub(crate) fn base_data_dir() -> PathBuf {
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("CodeCraft");
    }
    if let Some(home) = env::var_os("USERPROFILE").or_else(|| env::var_os("HOME")) {
        return PathBuf::from(home).join(".codecraft");
    }
    env::temp_dir().join("CodeCraft")
}

fn settings_path() -> PathBuf {
    base_data_dir().join("approval-settings.json")
}

pub(crate) fn load_settings() -> ApprovalSettings {
    if let Some(settings) = fs::read(settings_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    {
        return settings;
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct LegacyCodexHookSettings {
        enabled: bool,
        mode: String,
    }

    let migrated = fs::read(base_data_dir().join("codex-hook").join("settings.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<LegacyCodexHookSettings>(&bytes).ok())
        .filter(|settings| settings.enabled)
        .map(|settings| ApprovalSettings {
            mode: match settings.mode.as_str() {
                "permission" => ApprovalMode::Risk,
                "auto" => ApprovalMode::Automatic,
                _ => ApprovalMode::Manual,
            },
            minimal_mode: false,
        })
        .unwrap_or_default();
    let _ = save_settings(&migrated);
    migrated
}

pub(crate) fn save_settings(settings: &ApprovalSettings) -> Result<(), String> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(settings).map_err(|error| error.to_string())?;
    fs::write(path, bytes).map_err(|error| error.to_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ApprovalRisk {
    Low,
    Elevated,
    High,
}

pub(crate) fn should_auto_approve(mode: ApprovalMode, risk: ApprovalRisk) -> bool {
    match mode {
        ApprovalMode::Manual => false,
        ApprovalMode::Risk => risk == ApprovalRisk::Low,
        ApprovalMode::Automatic => true,
    }
}

/// Questions and plan confirmations require a subjective user decision even
/// when the global policy is set to automatic.
pub(crate) fn requires_user_decision(tool: &str) -> bool {
    let normalized = tool
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "askuserquestion"
            | "requestuserinput"
            | "question"
            | "plan"
            | "planexit"
            | "exitplanmode"
            | "updateplan"
    )
}

pub(crate) fn risk_for_tool(tool: &str, input: Option<&Value>) -> ApprovalRisk {
    match tool.to_ascii_lowercase().as_str() {
        "read" | "glob" | "grep" | "websearch" | "brainstorm" => ApprovalRisk::Low,
        "bash" | "shell" | "exec" | "command" => input
            .and_then(command_text)
            .map(|command| risk_for_command(&command))
            .unwrap_or(ApprovalRisk::Elevated),
        "write" | "edit" | "multiedit" | "notebookedit" | "webfetch" | "task" => {
            ApprovalRisk::Elevated
        }
        _ => ApprovalRisk::Elevated,
    }
}

pub(crate) fn risk_for_command(command: &str) -> ApprovalRisk {
    let normalized = command
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.is_empty() {
        return ApprovalRisk::Elevated;
    }

    const HIGH_RISK_MARKERS: [&str; 22] = [
        "rm -rf",
        "remove-item",
        " del ",
        " rmdir ",
        "rd /s",
        "format ",
        "diskpart",
        "git reset --hard",
        "git clean -f",
        "git checkout --",
        "git restore ",
        "git push --force",
        "git push -f",
        "git branch -d",
        "drop table",
        "truncate table",
        "delete from",
        "shutdown",
        "reboot",
        "stop-process",
        "taskkill",
        "kill -9",
    ];
    let padded = format!(" {normalized} ");
    if HIGH_RISK_MARKERS
        .iter()
        .any(|marker| padded.contains(marker))
    {
        return ApprovalRisk::High;
    }

    if normalized.contains('>')
        || normalized.contains("|")
        || normalized.contains("&&")
        || normalized.contains(';')
        || normalized.contains("sudo ")
        || normalized.contains("runas ")
        || normalized.contains("set-content")
        || normalized.contains("add-content")
        || normalized.contains("new-item")
        || normalized.contains("move-item")
        || normalized.contains("copy-item")
        || normalized.contains("git add")
        || normalized.contains("git commit")
        || normalized.contains("git push")
        || normalized.contains("git merge")
        || normalized.contains("git rebase")
        || normalized.contains("npm install")
        || normalized.contains("cargo install")
    {
        return ApprovalRisk::Elevated;
    }

    const LOW_RISK_PREFIXES: [&str; 32] = [
        "pwd",
        "cd ",
        "ls",
        "dir",
        "get-childitem",
        "get-location",
        "get-content",
        "select-string",
        "findstr",
        "rg",
        "grep",
        "where",
        "where.exe",
        "which",
        "type ",
        "cat ",
        "head ",
        "tail ",
        "git status",
        "git diff",
        "git log",
        "git show",
        "git branch",
        "git remote",
        "git rev-parse",
        "git ls-files",
        "cargo metadata",
        "cargo tree",
        "npm view",
        "npm list",
        "node --version",
        "python --version",
    ];
    let segments = normalized
        .split([',', '\n'])
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.len() > 1
        && segments.iter().all(|segment| {
            LOW_RISK_PREFIXES.iter().any(|prefix| {
                *segment == *prefix
                    || prefix.ends_with(' ') && segment.starts_with(prefix)
                    || segment.starts_with(&format!("{prefix} "))
            })
        })
    {
        return ApprovalRisk::Low;
    }
    if LOW_RISK_PREFIXES.iter().any(|prefix| {
        normalized == *prefix
            || prefix.ends_with(' ') && normalized.starts_with(prefix)
            || normalized.starts_with(&format!("{prefix} "))
    }) {
        ApprovalRisk::Low
    } else {
        ApprovalRisk::Elevated
    }
}

pub(crate) fn risk_for_claude_permission(payload: &Value) -> ApprovalRisk {
    let tool = payload
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    risk_for_tool(tool, payload.get("tool_input"))
}

pub(crate) fn risk_for_codex_hook(payload: &Value) -> ApprovalRisk {
    let tool = payload
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    risk_for_tool(tool, payload.get("tool_input"))
}

fn command_text(input: &Value) -> Option<String> {
    input
        .get("command")
        .and_then(Value::as_str)
        .or_else(|| input.as_str())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn risk_mode_only_approves_low_risk_actions() {
        assert!(should_auto_approve(ApprovalMode::Risk, ApprovalRisk::Low));
        assert!(!should_auto_approve(
            ApprovalMode::Risk,
            ApprovalRisk::Elevated
        ));
        assert!(!should_auto_approve(ApprovalMode::Risk, ApprovalRisk::High));
    }

    #[test]
    fn commands_are_classified_by_side_effect_risk() {
        assert_eq!(risk_for_command("git status"), ApprovalRisk::Low);
        assert_eq!(risk_for_command("npm install"), ApprovalRisk::Elevated);
        assert_eq!(
            risk_for_command("git reset --hard HEAD~1"),
            ApprovalRisk::High
        );
    }

    #[test]
    fn read_tools_are_low_risk_but_edits_require_review() {
        assert_eq!(
            risk_for_claude_permission(&json!({ "tool_name": "Read", "tool_input": {} })),
            ApprovalRisk::Low
        );
        assert_eq!(
            risk_for_claude_permission(&json!({ "tool_name": "Edit", "tool_input": {} })),
            ApprovalRisk::Elevated
        );
    }

    #[test]
    fn questions_and_plans_always_require_a_user_decision() {
        for tool in [
            "AskUserQuestion",
            "request_user_input",
            "question",
            "Plan",
            "plan_exit",
            "ExitPlanMode",
            "update_plan",
        ] {
            assert!(requires_user_decision(tool), "unexpected tool: {tool}");
        }
        assert!(!requires_user_decision("Bash"));
        assert!(!requires_user_decision("apply_patch"));
    }
}
