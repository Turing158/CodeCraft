//! Read-only native interactions. These records never authorize a tool or answer a card.
use crate::{files, protocol::*, wire};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

#[derive(Clone)]
struct PlanFile {
    path: PathBuf,
    hash: String,
    tool_use_id: String,
}

/// Ephemeral, session-local evidence. Cleared at every turn boundary and restart.
#[derive(Clone, Default)]
pub struct PlanFiles {
    pending: HashMap<String, String>,
    files: Vec<PlanFile>,
    overflowed: bool,
}

impl PlanFiles {
    pub fn observe(&mut self, input: &Value) {
        if !matches!(input["tool_name"].as_str(), Some("Write" | "Edit")) {
            return;
        }
        let Some(call) = input["tool_use_id"].as_str() else {
            return;
        };
        let Ok(fingerprint) = wire::hash(&json!([
            input["tool_name"],
            input["cwd"],
            input["tool_input"]
        ])) else {
            return;
        };
        if input["hook_event_name"] == "PreToolUse" {
            if self.pending.len() >= 256 {
                self.overflowed = true;
            } else {
                self.pending.entry(call.into()).or_insert(fingerprint);
            }
            return;
        }
        if input["hook_event_name"] != "PostToolUse"
            || self.pending.remove(call).as_ref() != Some(&fingerprint)
        {
            return;
        }
        // A denied tool can still emit PostToolUse with {}. Require an actual
        // successful change for the exact file and verify its resulting content.
        let response = &input["tool_response"];
        if response["is_error"] == true
            || response["isError"] == true
            || response
                .get("error")
                .is_some_and(|v| !v.is_null() && v != false && v != "")
        {
            return;
        }
        let Some(reference) = input["tool_input"]["file_path"].as_str() else {
            return;
        };
        let Some((path, body, hash)) = read_document(reference, input) else {
            return;
        };
        let confirmed = response["changes"].as_array().is_some_and(|changes| {
            changes.iter().any(|change| {
                let Some(changed_path) = change["file_path"].as_str() else {
                    return false;
                };
                let Some(content) = change["new_content"].as_str() else {
                    return false;
                };
                content.len() <= PLAN_LIMIT
                    && !body.trim().is_empty()
                    && change["file_action"] != "deleted"
                    && resolve_path(changed_path, input)
                        .canonicalize()
                        .ok()
                        .as_ref()
                        == Some(&path)
                    && files::normalize(content) == body
            })
        });
        if !confirmed {
            return;
        }
        if let Some(existing) = self.files.iter_mut().find(|f| f.path == path) {
            *existing = PlanFile {
                path,
                hash,
                tool_use_id: call.into(),
            };
        } else if self.files.len() < 16 {
            self.files.push(PlanFile {
                path,
                hash,
                tool_use_id: call.into(),
            });
        } else {
            self.overflowed = true;
        }
    }

    fn attach(&self, result: &mut Value, input: &Value) {
        if self.overflowed || self.files.len() > 1 {
            result["planSource"] = json!("ambiguous");
            return;
        }
        let Some(file) = self.files.first() else {
            return;
        };
        let Some((path, body, hash)) = read_document(&file.path.to_string_lossy(), input) else {
            return;
        };
        // Another session or an external editor may have replaced this file.
        if hash != file.hash {
            return;
        }
        result["documentPath"] = json!(path);
        result["plan"] = json!(body);
        result["planSource"] = json!("session_file");
        result["documentToolUseId"] = json!(file.tool_use_id);
    }
}

fn resolve_path(reference: &str, input: &Value) -> PathBuf {
    let path = Path::new(reference);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        Path::new(input["cwd"].as_str().unwrap_or("")).join(path)
    }
}

fn read_document(reference: &str, input: &Value) -> Option<(PathBuf, String, String)> {
    let path = resolve_path(reference, input).canonicalize().ok()?;
    for root in input["workspace_roots"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
    {
        let Ok(root) = Path::new(root).canonicalize() else {
            continue;
        };
        if [".trae/documents", ".trae/specs"]
            .iter()
            .any(|folder| path.starts_with(root.join(folder)))
        {
            let (body, hash) =
                files::plan(&path.to_string_lossy(), &root.to_string_lossy()).ok()?;
            return Some((path, body, hash));
        }
    }
    None
}

pub fn observation(
    session: &str,
    turn: u32,
    input: &Value,
    plan_files: Option<&PlanFiles>,
) -> Result<Option<Value>> {
    let kind = match (
        input["hook_event_name"].as_str(),
        input["tool_name"].as_str(),
        input["notification_type"].as_str(),
    ) {
        (Some("PreToolUse"), Some("AskUserQuestion"), _) => "question",
        (Some("Notification"), _, Some("ask_user_question")) => "question",
        (Some("Notification"), _, Some("document_review")) => "plan",
        _ => return Ok(None),
    };
    let args = input
        .get("tool_input")
        .filter(|v| v.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let message = input["message"]
        .as_str()
        .unwrap_or("")
        .chars()
        .take(8192)
        .collect::<String>();
    let tool_id = input["tool_use_id"].as_str();
    let request =
        wire::hash(&json!([session, turn, kind, tool_id, args, message])).map_err(invalid)?;
    let mut result = json!({"id":request,"kind":kind,"toolUseId":tool_id,"arguments":args,"message":message,"readOnly":true,"capturedAt":stamp(0),"documentPath":null,"plan":null});
    if kind == "plan" {
        // Explicit notification paths take precedence, even when invalid. Never
        // substitute a different document for a failed explicit reference.
        let reference = input["document_path"]
            .as_str()
            .or_else(|| input["documentPath"].as_str())
            .or_else(|| input["tool_input"]["document_path"].as_str());
        if let Some(reference) = reference {
            if let Some((path, body, _)) = read_document(reference, input) {
                result["documentPath"] = json!(path);
                result["plan"] = json!(body);
                result["planSource"] = json!("notification");
            }
        } else if let Some(plan_files) = plan_files {
            plan_files.attach(&mut result, input);
        }
    }
    Ok(Some(result))
}
