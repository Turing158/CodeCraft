//! Fail-closed capability evaluation for v4's P0 evidence gate. A prepared
//! probe, an installed file, or a synthetic test can never enable a capability.
use crate::protocol::{sha256, strict_json, MAX_MESSAGE_BYTES};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeIdentity {
    pub cursor_version: String,
    pub mode: String,
    pub platform: String,
    pub architecture: String,
    pub client_fingerprint: String,
    pub config_fingerprint: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    NotRun,
    Passed,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Evidence {
    pub path: String,
    pub sha256: String,
    pub kind: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Check {
    pub status: Status,
    pub verified_at: Option<String>,
    pub reason: String,
    pub evidence: Vec<Evidence>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Capability {
    HookOnly,
    NativeShared,
    AskUser,
    ReviewPlan,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Compatibility {
    pub schema_version: u32,
    pub identity: RuntimeIdentity,
    pub capture_scope: String,
    pub checks: BTreeMap<String, Check>,
    /// Each tool category has its own observed coverage evidence.
    pub tool_coverage: BTreeMap<String, Check>,
    pub qualified_tool_names: BTreeMap<String, String>,
    pub protocol: serde_json::Value,
    pub native_adapter: serde_json::Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityResult {
    pub capability: Capability,
    pub supported: bool,
    pub reasons: Vec<String>,
}

impl Compatibility {
    pub fn load(path: &Path) -> Result<Self, String> {
        let size = fs::metadata(path)
            .map_err(|_| "COMPATIBILITY_MISSING")?
            .len();
        if size > MAX_MESSAGE_BYTES as u64 {
            return Err("COMPATIBILITY_TOO_LARGE".into());
        }
        let bytes = fs::read(path).map_err(|_| "COMPATIBILITY_UNREADABLE")?;
        serde_json::from_value(strict_json(&bytes)?).map_err(|_| "INVALID_COMPATIBILITY".into())
    }

    pub fn evaluate(
        &self,
        runtime: &RuntimeIdentity,
        capability: Capability,
        tool: &str,
        directory: &Path,
    ) -> CapabilityResult {
        let mut reasons = Vec::new();
        if self.schema_version != 1 || self.capture_scope != "observedOnly" {
            reasons.push("INCOMPATIBLE_SCHEMA".into());
        }
        if runtime != &self.identity
            || runtime.platform != "windows"
            || !matches!(runtime.mode.as_str(), "Agent" | "Ask" | "Plan")
            || runtime.cursor_version.is_empty()
            || runtime.client_fingerprint.is_empty()
            || runtime.config_fingerprint.is_empty()
        {
            reasons.push("RUNTIME_IDENTITY_CHANGED_OR_UNVERIFIED".into());
        }
        let required: &[&str] = match capability {
            Capability::HookOnly => &["P0-01", "P0-03", "P0-05", "P0-07", "P0-08"],
            Capability::AskUser | Capability::ReviewPlan => &[
                "P0-01", "P0-02", "P0-03", "P0-04", "P0-05", "P0-06", "P0-07", "P0-08",
            ],
            Capability::NativeShared => &[
                "P0-01", "P0-03", "P0-05", "P0-07", "P0-08", "P0-09", "P0-10", "P0-11", "V27",
                "V28", "V29", "V30", "V31", "V32", "V33", "V34",
            ],
        };
        for name in required {
            if !self
                .checks
                .get(*name)
                .is_some_and(|check| valid_check(check, directory, runtime, name))
            {
                reasons.push(format!("{name}_NOT_VERIFIED"));
            }
        }
        if !self
            .tool_coverage
            .get(tool)
            .is_some_and(|check| valid_check(check, directory, runtime, &format!("tool:{tool}")))
        {
            reasons.push("TOOL_COVERAGE_NOT_VERIFIED".into());
        }
        if matches!(capability, Capability::AskUser | Capability::ReviewPlan)
            && !self.qualified_tool_names.get(tool).is_some_and(|name| {
                !name.is_empty() && name != tool && name != &format!("MCP:{tool}")
            })
        {
            reasons.push("QUALIFIED_TOOL_IDENTITY_NOT_VERIFIED".into());
        }
        CapabilityResult {
            capability,
            supported: reasons.is_empty(),
            reasons,
        }
    }
}

fn valid_check(check: &Check, directory: &Path, runtime: &RuntimeIdentity, check_id: &str) -> bool {
    check.status == Status::Passed
        && check.verified_at.as_ref().is_some_and(|v| !v.is_empty())
        && !check.evidence.is_empty()
        && check
            .evidence
            .iter()
            .all(|e| valid_evidence(e, directory, runtime, check_id))
}

fn valid_evidence(
    evidence: &Evidence,
    directory: &Path,
    runtime: &RuntimeIdentity,
    check_id: &str,
) -> bool {
    if evidence.kind != "cursorRuntime" || evidence.sha256.len() != 64 {
        return false;
    }
    let relative = Path::new(&evidence.path);
    if relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return false;
    }
    let Ok(root) = directory.canonicalize() else {
        return false;
    };
    let Ok(file) = directory.join(relative).canonicalize() else {
        return false;
    };
    // canonicalize also resolves junctions and symlinks out of the evidence directory.
    if !file.starts_with(root) {
        return false;
    }
    if !fs::metadata(&file).is_ok_and(|m| m.is_file() && m.len() <= MAX_MESSAGE_BYTES as u64) {
        return false;
    }
    let Ok(bytes) = fs::read(file) else {
        return false;
    };
    if sha256(&bytes) != evidence.sha256 {
        return false;
    }
    let Ok(value) = strict_json(&bytes) else {
        return false;
    };
    value["evidenceKind"] == "cursorRuntime"
        && value["cursorClientExecuted"] == true
        && value["probeInjected"] == false
        // A reviewed failure is still runtime evidence. Referencing it from a
        // check labelled "passed" must not turn that failure into support.
        && value["checkResults"][check_id] == "passed"
        && serde_json::from_value::<RuntimeIdentity>(value["identity"].clone())
            .is_ok_and(|identity| identity == *runtime)
        && value["checkIds"]
            .as_array()
            .is_some_and(|ids| ids.iter().any(|id| id == check_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn prepared_record_cannot_activate_any_capability() {
        let value = json!({"schemaVersion":1,"identity":{"cursorVersion":"3.21.13","mode":"Agent","platform":"windows","architecture":"x64","clientFingerprint":"a","configFingerprint":"b"},"captureScope":"observedOnly","checks":{},"toolCoverage":{},"qualifiedToolNames":{},"protocol":{},"nativeAdapter":{}});
        let record: Compatibility = serde_json::from_value(value).unwrap();
        for capability in [
            Capability::HookOnly,
            Capability::NativeShared,
            Capability::AskUser,
            Capability::ReviewPlan,
        ] {
            assert!(
                !record
                    .evaluate(&record.identity, capability, "Shell", Path::new("."))
                    .supported
            );
        }
    }
    #[test]
    fn failed_synthetic_missing_and_traversal_evidence_never_pass() {
        let runtime = RuntimeIdentity {
            cursor_version: "3.21.13".into(),
            mode: "Agent".into(),
            platform: "windows".into(),
            architecture: "x64".into(),
            client_fingerprint: "client".into(),
            config_fingerprint: "config".into(),
        };
        let mut evidence = Evidence {
            path: "../escape.json".into(),
            sha256: "a".repeat(64),
            kind: "cursorRuntime".into(),
        };
        assert!(!valid_evidence(
            &evidence,
            Path::new("."),
            &runtime,
            "P0-01"
        ));
        evidence.path = "missing.json".into();
        assert!(!valid_evidence(
            &evidence,
            Path::new("."),
            &runtime,
            "P0-01"
        ));
        evidence.kind = "synthetic".into();
        assert!(!valid_evidence(
            &evidence,
            Path::new("."),
            &runtime,
            "P0-01"
        ));
        let check = Check {
            status: Status::Failed,
            verified_at: Some("2026-09-21".into()),
            reason: String::new(),
            evidence: vec![],
        };
        assert!(!valid_check(&check, Path::new("."), &runtime, "P0-01"));
    }
}
