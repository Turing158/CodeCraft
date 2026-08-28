//! Hook-only Codex session and interaction state.

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_OUTPUT_ENTRIES: usize = 24;
const MAX_OUTPUT_ENTRY_CHARS: usize = 8_000;
const MAX_INTERACTIONS_PER_SESSION: usize = 8;
const MAX_ACTIVITIES: usize = 40;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn cap_text(text: &str) -> String {
    let mut chars = text.chars();
    let mut capped: String = chars.by_ref().take(MAX_OUTPUT_ENTRY_CHARS).collect();
    if chars.next().is_some() {
        capped.push('\u{2026}');
    }
    capped
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CodexSessionStatus {
    Working,
    WaitingForInput,
    WaitingForApproval,
    Idle,
    Stopped,
}

impl CodexSessionStatus {
    fn is_active(self) -> bool {
        !matches!(self, Self::Idle | Self::Stopped)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CodexInteractionKind {
    PermissionsApproval,
    UserInput,
    Plan,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CodexApprovalDecision {
    Accept,
    AcceptForSession,
    Decline,
    Cancel,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexActivity {
    pub id: String,
    pub tool: String,
    pub summary: String,
    pub status: String,
    pub started_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexOutputEntry {
    pub id: String,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexQuestionOption {
    pub label: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexQuestion {
    pub id: String,
    pub header: String,
    pub question: String,
    pub options: Vec<CodexQuestionOption>,
    pub is_other: bool,
    pub is_secret: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexInteraction {
    pub request_id: String,
    pub kind: CodexInteractionKind,
    pub answerable: bool,
    pub thread_id: String,
    pub title: String,
    pub detail: String,
    pub plan: Option<String>,
    pub questions: Vec<CodexQuestion>,
    pub allow_session: bool,
    pub is_secret: bool,
    pub resolved: bool,
    pub captured_at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexSession {
    pub id: String,
    pub status: CodexSessionStatus,
    pub title: String,
    pub cwd: Option<String>,
    pub started_at: u64,
    pub updated_at: u64,
    pub activities: Vec<CodexActivity>,
    pub outputs: Vec<CodexOutputEntry>,
    pub pending_interaction_id: Option<String>,
}

impl CodexSession {
    pub(crate) fn is_active(&self) -> bool {
        self.status.is_active()
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexSnapshot {
    pub connected: bool,
    pub integration_error: Option<String>,
    pub version: u64,
    pub sessions: Vec<CodexSession>,
    pub interactions: Vec<CodexInteraction>,
}

#[derive(Clone, Debug)]
pub(crate) enum CodexEvent {
    HookSessionStarted {
        id: String,
        cwd: Option<String>,
        title: Option<String>,
    },
    HookSessionEnded {
        id: String,
    },
    HookPrompt {
        thread_id: String,
        text: String,
    },
    HookApproval {
        request_id: String,
        thread_id: String,
        tool: String,
        summary: String,
        cwd: Option<String>,
        allow_session: bool,
    },
    HookUserInput {
        request_id: String,
        thread_id: String,
        questions: Vec<CodexQuestion>,
        cwd: Option<String>,
    },
    HookPlan {
        request_id: String,
        thread_id: String,
        plan: String,
        cwd: Option<String>,
    },
    HookToolStarted {
        thread_id: String,
        activity_id: String,
        tool: String,
        summary: String,
    },
    HookToolFinished {
        thread_id: String,
        activity_id: String,
        tool: String,
        summary: String,
        failed: bool,
    },
    HookStopped {
        thread_id: String,
        reason: Option<String>,
    },
    HookSubagentStopped {
        thread_id: String,
    },
}

pub(crate) fn parse_user_input_questions(questions: Option<&Vec<Value>>) -> Vec<CodexQuestion> {
    let Some(questions) = questions else {
        return vec![CodexQuestion {
            id: "answer".to_string(),
            header: "Request user input".to_string(),
            question: "Codex is requesting information from you.".to_string(),
            options: Vec::new(),
            is_other: true,
            is_secret: false,
        }];
    };

    let parsed: Vec<CodexQuestion> = questions
        .iter()
        .enumerate()
        .map(|(index, question)| {
            let id = question
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("question-{index}"));
            let header = question
                .get("header")
                .and_then(Value::as_str)
                .filter(|header| !header.is_empty())
                .unwrap_or("Request user input")
                .to_string();
            let text = question
                .get("question")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .unwrap_or("Codex is requesting information from you.")
                .to_string();
            let options = question
                .get("options")
                .and_then(Value::as_array)
                .map(|options| {
                    options
                        .iter()
                        .filter_map(|option| {
                            let label = option.get("label").and_then(Value::as_str)?;
                            Some(CodexQuestionOption {
                                label: label.to_string(),
                                description: option
                                    .get("description")
                                    .and_then(Value::as_str)
                                    .map(str::to_owned),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            CodexQuestion {
                id,
                header,
                question: text,
                options,
                is_other: question
                    .get("isOther")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                is_secret: question
                    .get("isSecret")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            }
        })
        .collect();

    if parsed.is_empty() {
        return parse_user_input_questions(None);
    }
    parsed
}

pub(crate) struct CodexStore {
    connected: bool,
    error: Option<String>,
    version: u64,
    sessions: HashMap<String, CodexSession>,
    interactions: HashMap<String, CodexInteraction>,
    responded: HashSet<String>,
}

impl Default for CodexStore {
    fn default() -> Self {
        Self {
            connected: false,
            error: None,
            version: 0,
            sessions: HashMap::new(),
            interactions: HashMap::new(),
            responded: HashSet::new(),
        }
    }
}

impl CodexStore {
    /// Drop all hook-derived state when the integration is unavailable.
    ///
    /// Hook events are kept only in memory, but leaving them around after an
    /// uninstall would make old Codex sessions reappear until the process is
    /// restarted (and could also keep them eligible for auto-reveal).
    pub(crate) fn clear(&mut self) {
        if self.sessions.is_empty() && self.interactions.is_empty() && self.responded.is_empty() {
            return;
        }
        self.sessions.clear();
        self.interactions.clear();
        self.responded.clear();
        self.bump();
    }

    pub(crate) fn set_integration_error(&mut self, error: Option<String>) {
        let connected = error.is_none();
        if self.connected == connected && self.error == error {
            return;
        }
        self.connected = connected;
        self.error = error;
        self.bump();
    }

    pub(crate) fn apply(&mut self, event: CodexEvent) {
        match event {
            CodexEvent::HookSessionStarted { id, cwd, title } => {
                self.ensure_session(
                    &id,
                    title.unwrap_or_else(|| "Codex 会话".to_string()),
                    cwd,
                    false,
                );
            }
            CodexEvent::HookSessionEnded { id } => {
                self.resolve_interactions_for_thread(&id);
                if let Some(session) = self.sessions.get_mut(&id) {
                    session.status = CodexSessionStatus::Stopped;
                    session.updated_at = now_ms();
                }
            }
            CodexEvent::HookPrompt { thread_id, text } => {
                self.resolve_non_approval_interactions_for_thread(&thread_id);
                let session = self.ensure_session(&thread_id, "Codex 会话".to_string(), None, true);
                let id = format!("hook-prompt-{}", now_ms());
                push_output(session, &id, &format!("用户：{text}"));
                session.updated_at = now_ms();
            }
            CodexEvent::HookApproval {
                request_id,
                thread_id,
                tool,
                summary,
                cwd,
                allow_session,
            } => {
                self.ensure_session(&thread_id, "Codex 会话".to_string(), cwd.clone(), true);
                let detail = match (summary.trim(), cwd.as_deref()) {
                    (summary, Some(cwd)) if !summary.is_empty() => {
                        format!("{summary}\n\nCWD: {cwd}")
                    }
                    (summary, _) if !summary.is_empty() => summary.to_string(),
                    (_, Some(cwd)) => format!("CWD: {cwd}"),
                    _ => "Codex 请求执行操作".to_string(),
                };
                self.add_interaction(CodexInteraction {
                    request_id,
                    kind: CodexInteractionKind::PermissionsApproval,
                    answerable: true,
                    thread_id: thread_id.clone(),
                    title: format!("Codex · {tool} 需要审批"),
                    detail,
                    plan: None,
                    questions: Vec::new(),
                    allow_session,
                    is_secret: false,
                    resolved: false,
                    captured_at: now_ms(),
                });
                if let Some(session) = self.sessions.get_mut(&thread_id) {
                    session.status = CodexSessionStatus::WaitingForApproval;
                    session.updated_at = now_ms();
                }
            }
            CodexEvent::HookUserInput {
                request_id,
                thread_id,
                questions,
                cwd,
            } => {
                self.resolve_non_approval_interactions_for_thread(&thread_id);
                self.ensure_session(&thread_id, "Codex 外部会话".to_string(), cwd, false);
                let is_secret = questions.iter().any(|question| question.is_secret);
                self.add_interaction(CodexInteraction {
                    request_id,
                    kind: CodexInteractionKind::UserInput,
                    answerable: false,
                    thread_id: thread_id.clone(),
                    title: "外部 Codex 会话正在等待回答".to_string(),
                    detail: "请在原 Codex 终端或桌面任务中完成回答。".to_string(),
                    plan: None,
                    questions,
                    allow_session: false,
                    is_secret,
                    resolved: false,
                    captured_at: now_ms(),
                });
                if let Some(session) = self.sessions.get_mut(&thread_id) {
                    session.status = CodexSessionStatus::WaitingForInput;
                    session.updated_at = now_ms();
                }
            }
            CodexEvent::HookPlan {
                request_id,
                thread_id,
                plan,
                cwd,
            } => {
                self.resolve_non_approval_interactions_for_thread(&thread_id);
                self.ensure_session(&thread_id, "Codex 外部会话".to_string(), cwd.clone(), false);
                self.add_interaction(CodexInteraction {
                    request_id,
                    kind: CodexInteractionKind::Plan,
                    answerable: false,
                    thread_id: thread_id.clone(),
                    title: "Codex · 实行计划".to_string(),
                    detail: "计划已生成，请前往原 Codex 界面选择是否实行。".to_string(),
                    plan: Some(plan),
                    questions: Vec::new(),
                    allow_session: false,
                    is_secret: false,
                    resolved: false,
                    captured_at: now_ms(),
                });
                if let Some(session) = self.sessions.get_mut(&thread_id) {
                    session.status = CodexSessionStatus::WaitingForInput;
                    session.updated_at = now_ms();
                }
            }
            CodexEvent::HookToolStarted {
                thread_id,
                activity_id,
                tool,
                summary,
            } => {
                self.resolve_non_approval_interactions_for_thread(&thread_id);
                let session = self.ensure_session(&thread_id, "Codex 会话".to_string(), None, true);
                upsert_activity(session, &activity_id, &tool, &summary, "running");
                session.status = CodexSessionStatus::Working;
                session.updated_at = now_ms();
            }
            CodexEvent::HookToolFinished {
                thread_id,
                activity_id,
                tool,
                summary,
                failed,
            } => {
                if tool.eq_ignore_ascii_case("request_user_input") {
                    self.resolve_hook_user_input(&thread_id, &activity_id);
                }
                let session =
                    self.ensure_session(&thread_id, "Codex 会话".to_string(), None, false);
                upsert_activity(
                    session,
                    &activity_id,
                    &tool,
                    &summary,
                    if failed { "failed" } else { "completed" },
                );
                session.updated_at = now_ms();
            }
            CodexEvent::HookStopped { thread_id, reason } => {
                self.resolve_interactions_for_thread(&thread_id);
                if let Some(session) = self.sessions.get_mut(&thread_id) {
                    session.status = CodexSessionStatus::Idle;
                    if let Some(text) = reason.filter(|text| !text.trim().is_empty()) {
                        let id = format!("hook-assistant-{}", now_ms());
                        push_output(session, &id, &format!("Codex：{text}"));
                    }
                    session.updated_at = now_ms();
                }
            }
            CodexEvent::HookSubagentStopped { thread_id } => {
                if let Some(session) = self.sessions.get_mut(&thread_id) {
                    session.updated_at = now_ms();
                }
            }
        }
        self.bump();
    }

    fn ensure_session(
        &mut self,
        id: &str,
        title: String,
        cwd: Option<String>,
        mark_working: bool,
    ) -> &mut CodexSession {
        let title_is_meaningful = !title.is_empty() && title != "Codex 会话";
        let insert_title = if title.is_empty() {
            id.to_string()
        } else {
            title.clone()
        };
        let session = self
            .sessions
            .entry(id.to_string())
            .or_insert_with(|| CodexSession {
                id: id.to_string(),
                status: if mark_working {
                    CodexSessionStatus::Working
                } else {
                    CodexSessionStatus::Idle
                },
                title: insert_title,
                cwd: cwd.clone(),
                started_at: now_ms(),
                updated_at: now_ms(),
                activities: Vec::new(),
                outputs: Vec::new(),
                pending_interaction_id: None,
            });
        if title_is_meaningful
            && (session.title == session.id
                || session.title == "Codex 会话"
                || session.title == "Codex 外部会话")
        {
            session.title = title;
        }
        if session.cwd.is_none() && cwd.is_some() {
            session.cwd = cwd;
        }
        if mark_working
            && !matches!(
                session.status,
                CodexSessionStatus::WaitingForInput | CodexSessionStatus::WaitingForApproval
            )
        {
            session.status = CodexSessionStatus::Working;
        }
        session
    }

    fn add_interaction(&mut self, interaction: CodexInteraction) {
        let request_id = interaction.request_id.clone();
        let thread_id = interaction.thread_id.clone();
        if self.interactions.contains_key(&request_id) {
            return;
        }
        self.interactions.insert(request_id.clone(), interaction);
        if let Some(session) = self.sessions.get_mut(&thread_id) {
            session.pending_interaction_id = Some(request_id);
        }
        self.prune_interactions(&thread_id);
    }

    fn prune_interactions(&mut self, thread_id: &str) {
        let mut ids: Vec<(String, u64)> = self
            .interactions
            .values()
            .filter(|interaction| interaction.thread_id == thread_id)
            .map(|interaction| (interaction.request_id.clone(), interaction.captured_at))
            .collect();
        if ids.len() <= MAX_INTERACTIONS_PER_SESSION {
            return;
        }
        ids.sort_by_key(|(_, captured_at)| *captured_at);
        let excess = ids.len().saturating_sub(MAX_INTERACTIONS_PER_SESSION);
        for (request_id, _) in ids.into_iter().take(excess) {
            if self
                .interactions
                .get(&request_id)
                .is_some_and(|interaction| interaction.resolved)
            {
                self.interactions.remove(&request_id);
                self.responded.remove(&request_id);
            }
        }
    }

    fn resolve_interactions_for_thread(&mut self, thread_id: &str) {
        let request_ids: Vec<String> = self
            .interactions
            .values()
            .filter(|interaction| interaction.thread_id == thread_id && !interaction.resolved)
            .map(|interaction| interaction.request_id.clone())
            .collect();
        for request_id in request_ids {
            self.mark_interaction_resolved(&request_id);
        }
    }

    fn resolve_hook_user_input(&mut self, thread_id: &str, activity_id: &str) {
        let exact_id = format!("hook-input-{activity_id}");
        let request_id = if self.interactions.get(&exact_id).is_some_and(|interaction| {
            interaction.thread_id == thread_id
                && interaction.kind == CodexInteractionKind::UserInput
                && !interaction.resolved
        }) {
            Some(exact_id)
        } else {
            self.interactions
                .values()
                .filter(|interaction| {
                    interaction.thread_id == thread_id
                        && interaction.kind == CodexInteractionKind::UserInput
                        && !interaction.resolved
                })
                .max_by_key(|interaction| interaction.captured_at)
                .map(|interaction| interaction.request_id.clone())
        };
        if let Some(request_id) = request_id {
            self.mark_interaction_resolved(&request_id);
        }
    }

    fn resolve_non_approval_interactions_for_thread(&mut self, thread_id: &str) {
        let request_ids: Vec<String> = self
            .interactions
            .values()
            .filter(|interaction| {
                interaction.thread_id == thread_id
                    && !interaction.resolved
                    && interaction.kind != CodexInteractionKind::PermissionsApproval
            })
            .map(|interaction| interaction.request_id.clone())
            .collect();
        for request_id in request_ids {
            self.mark_interaction_resolved(&request_id);
        }
    }

    pub(crate) fn resolve_hook_approval(&mut self, request_id: &str) -> Result<bool, String> {
        if self.responded.contains(request_id) {
            return Ok(false);
        }
        let interaction = self
            .interactions
            .get(request_id)
            .ok_or_else(|| format!("unknown approval request: {request_id}"))?;
        if interaction.kind != CodexInteractionKind::PermissionsApproval {
            return Err(format!("request is not an approval: {request_id}"));
        }
        self.responded.insert(request_id.to_string());
        self.mark_interaction_resolved(request_id);
        Ok(true)
    }

    pub(crate) fn hook_approval_is_pending(&self, request_id: &str) -> bool {
        !self.responded.contains(request_id)
            && self
                .interactions
                .get(request_id)
                .is_some_and(|interaction| {
                    interaction.kind == CodexInteractionKind::PermissionsApproval
                        && !interaction.resolved
                })
    }

    fn mark_interaction_resolved(&mut self, request_id: &str) {
        let Some(thread_id) = self.interactions.get_mut(request_id).map(|interaction| {
            interaction.resolved = true;
            interaction.thread_id.clone()
        }) else {
            return;
        };

        let next_pending = self
            .interactions
            .values()
            .filter(|interaction| interaction.thread_id == thread_id && !interaction.resolved)
            .max_by_key(|interaction| interaction.captured_at)
            .map(|interaction| interaction.request_id.clone());
        if let Some(session) = self.sessions.get_mut(&thread_id) {
            session.pending_interaction_id = next_pending;
            if session.pending_interaction_id.is_none()
                && matches!(
                    session.status,
                    CodexSessionStatus::WaitingForInput | CodexSessionStatus::WaitingForApproval
                )
            {
                session.status = CodexSessionStatus::Working;
            }
            session.updated_at = now_ms();
        }
    }

    fn bump(&mut self) {
        self.version = self.version.wrapping_add(1);
    }

    pub(crate) fn snapshot(&self) -> CodexSnapshot {
        let mut sessions: Vec<CodexSession> = self.sessions.values().cloned().collect();
        sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        let mut interactions: Vec<CodexInteraction> = self.interactions.values().cloned().collect();
        interactions.sort_by_key(|interaction| interaction.captured_at);
        CodexSnapshot {
            connected: self.connected,
            integration_error: self.error.clone(),
            version: self.version,
            sessions,
            interactions,
        }
    }
}

fn push_output(session: &mut CodexSession, id: &str, text: &str) {
    if let Some(entry) = session.outputs.iter_mut().find(|entry| entry.id == id) {
        let mut merged = entry.text.clone();
        merged.push_str(text);
        entry.text = cap_text(&merged);
        return;
    }
    session.outputs.push(CodexOutputEntry {
        id: id.to_string(),
        text: cap_text(text),
    });
    if session.outputs.len() > MAX_OUTPUT_ENTRIES {
        let excess = session.outputs.len() - MAX_OUTPUT_ENTRIES;
        session.outputs.drain(..excess);
    }
}

fn upsert_activity(session: &mut CodexSession, id: &str, tool: &str, summary: &str, status: &str) {
    let now = now_ms();
    if let Some(activity) = session
        .activities
        .iter_mut()
        .find(|activity| activity.id == id)
    {
        activity.tool = tool.to_string();
        activity.summary = cap_text(summary);
        activity.status = status.to_string();
        activity.updated_at = now;
        return;
    }
    session.activities.push(CodexActivity {
        id: id.to_string(),
        tool: tool.to_string(),
        summary: cap_text(summary),
        status: status.to_string(),
        started_at: now,
        updated_at: now,
    });
    if session.activities.len() > MAX_ACTIVITIES {
        let excess = session.activities.len() - MAX_ACTIVITIES;
        session.activities.drain(..excess);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_session_tracks_prompt_tools_and_stop_output() {
        let mut store = CodexStore::default();
        store.set_integration_error(None);
        store.apply(CodexEvent::HookSessionStarted {
            id: "session-1".to_string(),
            cwd: Some("C:\\work".to_string()),
            title: Some("Hook task".to_string()),
        });
        store.apply(CodexEvent::HookPrompt {
            thread_id: "session-1".to_string(),
            text: "Inspect the project".to_string(),
        });
        store.apply(CodexEvent::HookToolStarted {
            thread_id: "session-1".to_string(),
            activity_id: "read-1".to_string(),
            tool: "Read".to_string(),
            summary: "src/main.ts".to_string(),
        });
        store.apply(CodexEvent::HookToolFinished {
            thread_id: "session-1".to_string(),
            activity_id: "read-1".to_string(),
            tool: "Read".to_string(),
            summary: "src/main.ts".to_string(),
            failed: false,
        });
        store.apply(CodexEvent::HookStopped {
            thread_id: "session-1".to_string(),
            reason: Some("Done".to_string()),
        });

        let snapshot = store.snapshot();
        assert!(snapshot.connected);
        assert_eq!(snapshot.sessions.len(), 1);
        assert_eq!(snapshot.sessions[0].status, CodexSessionStatus::Idle);
        assert_eq!(snapshot.sessions[0].activities[0].status, "completed");
        assert!(snapshot.sessions[0]
            .outputs
            .iter()
            .any(|output| output.text.contains("Done")));
    }

    #[test]
    fn hook_approval_can_be_resolved_once() {
        let mut store = CodexStore::default();
        store.apply(CodexEvent::HookApproval {
            request_id: "hook-permission-1".to_string(),
            thread_id: "session-1".to_string(),
            tool: "Shell".to_string(),
            summary: "npm test".to_string(),
            cwd: None,
            allow_session: true,
        });

        assert!(store.hook_approval_is_pending("hook-permission-1"));
        assert!(store.resolve_hook_approval("hook-permission-1").unwrap());
        assert!(!store.resolve_hook_approval("hook-permission-1").unwrap());
        assert!(!store.hook_approval_is_pending("hook-permission-1"));
    }

    #[test]
    fn clear_removes_hook_sessions_and_interactions() {
        let mut store = CodexStore::default();
        store.apply(CodexEvent::HookApproval {
            request_id: "hook-permission-clear".to_string(),
            thread_id: "session-clear".to_string(),
            tool: "Shell".to_string(),
            summary: "dir".to_string(),
            cwd: None,
            allow_session: false,
        });

        store.clear();
        let snapshot = store.snapshot();
        assert!(snapshot.sessions.is_empty());
        assert!(snapshot.interactions.is_empty());
    }

    #[test]
    fn hook_user_input_is_read_only_and_resolves_after_tool_completion() {
        let mut store = CodexStore::default();
        store.apply(CodexEvent::HookUserInput {
            request_id: "hook-input-question-1".to_string(),
            thread_id: "session-1".to_string(),
            questions: parse_user_input_questions(Some(&vec![serde_json::json!({
                "id": "q1",
                "header": "Mode",
                "question": "Choose a mode",
                "options": [{ "label": "Fast", "description": "Move quickly" }]
            })])),
            cwd: None,
        });

        let interaction = &store.snapshot().interactions[0];
        assert_eq!(interaction.kind, CodexInteractionKind::UserInput);
        assert!(!interaction.answerable);

        store.apply(CodexEvent::HookToolFinished {
            thread_id: "session-1".to_string(),
            activity_id: "question-1".to_string(),
            tool: "request_user_input".to_string(),
            summary: "answered".to_string(),
            failed: false,
        });
        assert!(store.snapshot().interactions[0].resolved);
    }

    #[test]
    fn newer_hook_user_input_replaces_the_previous_pending_question() {
        let mut store = CodexStore::default();
        let questions = parse_user_input_questions(Some(&vec![serde_json::json!({
            "id": "q1",
            "header": "Mode",
            "question": "Choose a mode"
        })]));
        store.apply(CodexEvent::HookUserInput {
            request_id: "hook-input-question-1".to_string(),
            thread_id: "session-1".to_string(),
            questions: questions.clone(),
            cwd: None,
        });
        store.apply(CodexEvent::HookUserInput {
            request_id: "hook-input-question-2".to_string(),
            thread_id: "session-1".to_string(),
            questions,
            cwd: None,
        });

        let snapshot = store.snapshot();
        assert!(snapshot
            .interactions
            .iter()
            .find(|interaction| interaction.request_id == "hook-input-question-1")
            .is_some_and(|interaction| interaction.resolved));
        assert!(snapshot
            .interactions
            .iter()
            .find(|interaction| interaction.request_id == "hook-input-question-2")
            .is_some_and(|interaction| !interaction.resolved));
        assert_eq!(
            snapshot.sessions[0].pending_interaction_id.as_deref(),
            Some("hook-input-question-2")
        );
    }

    #[test]
    fn hook_plan_waits_for_input_and_resolves_when_work_resumes() {
        let mut store = CodexStore::default();
        store.apply(CodexEvent::HookPlan {
            request_id: "hook-plan-session-1-turn-1".to_string(),
            thread_id: "session-1".to_string(),
            plan: "## Plan\n\n1. Inspect\n2. Implement".to_string(),
            cwd: Some("C:/repo".to_string()),
        });

        let snapshot = store.snapshot();
        let interaction = &snapshot.interactions[0];
        assert_eq!(interaction.kind, CodexInteractionKind::Plan);
        assert!(!interaction.answerable);
        assert_eq!(
            interaction.plan.as_deref(),
            Some("## Plan\n\n1. Inspect\n2. Implement")
        );
        assert_eq!(
            snapshot.sessions[0].status,
            CodexSessionStatus::WaitingForInput
        );

        store.apply(CodexEvent::HookToolStarted {
            thread_id: "session-1".to_string(),
            activity_id: "tool-1".to_string(),
            tool: "apply_patch".to_string(),
            summary: "Edit files".to_string(),
        });

        assert!(store.snapshot().interactions[0].resolved);
    }
}
