use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{mpsc, OnceLock},
    thread,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::approval_policy;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SoundEvent {
    ToolCall,
    PermissionApproval,
    PlanExecution,
    SessionSuccess,
    SessionFailure,
}

impl SoundEvent {
    fn file_stem(self) -> &'static str {
        match self {
            Self::ToolCall => "tool-call",
            Self::PermissionApproval => "permission-approval",
            Self::PlanExecution => "plan-execution",
            Self::SessionSuccess => "session-success",
            Self::SessionFailure => "session-failure",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SoundPack {
    #[default]
    NoteBlock,
    Cat,
    BoneBlock,
    Experience,
    Custom,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct NativeSoundSettings {
    pub enabled: bool,
    pub volume: f32,
    pub pack: SoundPack,
    pub custom_files: HashMap<String, String>,
}

impl Default for NativeSoundSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            volume: 0.7,
            pack: SoundPack::NoteBlock,
            custom_files: HashMap::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SoundFrame {
    status: String,
    activity_ids: HashSet<String>,
    failed_activity_ids: HashSet<String>,
    permission_id: Option<String>,
    plan_id: Option<String>,
}

#[derive(Default)]
pub(crate) struct NativeSoundObserver {
    frames: HashMap<String, SoundFrame>,
    primed_sources: HashSet<String>,
}

struct PlaybackRequest {
    bytes: Vec<u8>,
    volume: f32,
}

static AUDIO_SENDER: OnceLock<mpsc::Sender<PlaybackRequest>> = OnceLock::new();

fn settings_path() -> PathBuf {
    approval_policy::base_data_dir().join("sound-settings.json")
}

fn custom_sound_dir() -> PathBuf {
    approval_policy::base_data_dir().join("sounds")
}

pub(crate) fn load_settings() -> NativeSoundSettings {
    fs::read(settings_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub(crate) fn save_settings(settings: &NativeSoundSettings) -> Result<(), String> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(settings).map_err(|error| error.to_string())?;
    fs::write(path, bytes).map_err(|error| error.to_string())
}

pub(crate) fn save_custom_sound(
    event: SoundEvent,
    file_name: &str,
    bytes: &[u8],
    settings: &mut NativeSoundSettings,
) -> Result<(), String> {
    let extension = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| {
            value
                .chars()
                .all(|character| character.is_ascii_alphanumeric())
        })
        .unwrap_or("audio")
        .to_ascii_lowercase();
    let directory = custom_sound_dir();
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let path = directory.join(format!("{}.{}", event.file_stem(), extension));
    fs::write(&path, bytes).map_err(|error| error.to_string())?;

    settings.custom_files.insert(
        event.file_stem().to_string(),
        path.to_string_lossy().into_owned(),
    );
    save_settings(settings)
}

fn preset_bytes(pack: SoundPack, completion: bool) -> &'static [u8] {
    match (pack, completion) {
        (SoundPack::Cat, false) => include_bytes!("../sound/approval-cat.ogg"),
        (SoundPack::Cat, true) => include_bytes!("../sound/task-complete-cat.ogg"),
        (SoundPack::BoneBlock, false) => include_bytes!("../sound/approval-bone-block.ogg"),
        (SoundPack::BoneBlock, true) => {
            include_bytes!("../sound/task-complete-bone-block.ogg")
        }
        (SoundPack::Experience, false) => include_bytes!("../sound/approval-experience.ogg"),
        (SoundPack::Experience, true) => {
            include_bytes!("../sound/task-complete-experience.ogg")
        }
        (_, false) => include_bytes!("../sound/approval-note-block.ogg"),
        (_, true) => include_bytes!("../sound/task-complete-note-block.ogg"),
    }
}

fn event_bytes(settings: &NativeSoundSettings, event: SoundEvent) -> Option<Vec<u8>> {
    if matches!(settings.pack, SoundPack::Custom) {
        let path = settings.custom_files.get(event.file_stem())?;
        return fs::read(path).ok();
    }
    let completion = matches!(
        event,
        SoundEvent::SessionSuccess | SoundEvent::SessionFailure
    );
    Some(preset_bytes(settings.pack, completion).to_vec())
}

pub(crate) fn play(event: SoundEvent) {
    let settings = load_settings();
    if !settings.enabled {
        return;
    }
    let Some(bytes) = event_bytes(&settings, event) else {
        return;
    };
    let volume = settings.volume.clamp(0.0, 1.0);
    let sender = AUDIO_SENDER.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<PlaybackRequest>();
        thread::spawn(move || {
            let Ok(stream) = rodio::DeviceSinkBuilder::open_default_sink() else {
                return;
            };
            for request in receiver {
                let Ok(player) = rodio::play(stream.mixer(), Cursor::new(request.bytes)) else {
                    continue;
                };
                player.set_volume(request.volume);
                player.sleep_until_end();
            }
        });
        sender
    });
    let _ = sender.send(PlaybackRequest { bytes, volume });
}

fn string_set(value: Option<&Value>, field: &str) -> HashSet<String> {
    value
        .and_then(|value| value.get(field))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("id").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

fn failed_string_set(value: Option<&Value>) -> HashSet<String> {
    value
        .and_then(|value| value.get("activities"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("status").and_then(Value::as_str) == Some("failed"))
        .filter_map(|item| item.get("id").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

fn frame_for(source: &str, session: &Value, interactions: &[Value]) -> SoundFrame {
    let status = session
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let interaction = session
        .get("id")
        .and_then(Value::as_str)
        .and_then(|session_id| {
            interactions.iter().find(|interaction| {
                interaction.get("threadId").and_then(Value::as_str) == Some(session_id)
                    && interaction.get("resolved").and_then(Value::as_bool) != Some(true)
            })
        });
    let review = session
        .get("pendingReviews")
        .and_then(Value::as_array)
        .and_then(|reviews| reviews.first());

    let permission_id = match source {
        "claude" | "pi" | "dsh" => session
            .get("permission")
            .and_then(|value| value.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        "codex" => interaction
            .filter(|value| {
                value.get("kind").and_then(Value::as_str) == Some("permissionsApproval")
            })
            .and_then(|value| value.get("requestId"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        "opencode" => review
            .filter(|value| {
                matches!(
                    value.get("reviewType").and_then(Value::as_str),
                    Some("nativePermission" | "strictToolGate")
                )
            })
            .and_then(|value| value.get("reviewId"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        _ => None,
    };
    let plan_id = match source {
        "claude" | "dsh" => session
            .get("plan")
            .and_then(|value| value.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        "codex" => interaction
            .filter(|value| value.get("kind").and_then(Value::as_str) == Some("plan"))
            .and_then(|value| value.get("requestId"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        _ => None,
    };

    SoundFrame {
        status,
        activity_ids: string_set(Some(session), "activities"),
        failed_activity_ids: failed_string_set(Some(session)),
        permission_id,
        plan_id,
    }
}

fn has_new(previous: Option<&HashSet<String>>, next: &HashSet<String>) -> bool {
    next.iter()
        .any(|value| previous.is_none_or(|previous| !previous.contains(value)))
}

fn transition_events(previous: Option<&SoundFrame>, next: &SoundFrame) -> Vec<SoundEvent> {
    let mut events = Vec::new();
    if has_new(
        previous.map(|frame| &frame.activity_ids),
        &next.activity_ids,
    ) {
        events.push(SoundEvent::ToolCall);
    }
    if next.permission_id.is_some()
        && next.permission_id.as_ref() != previous.and_then(|frame| frame.permission_id.as_ref())
    {
        events.push(SoundEvent::PermissionApproval);
    }
    if next.plan_id.is_some()
        && next.plan_id.as_ref() != previous.and_then(|frame| frame.plan_id.as_ref())
    {
        events.push(SoundEvent::PlanExecution);
    }

    let failed_activity_added = has_new(
        previous.map(|frame| &frame.failed_activity_ids),
        &next.failed_activity_ids,
    );
    let session_failed =
        next.status == "toolFailed" && previous.is_none_or(|frame| frame.status != "toolFailed");
    if failed_activity_added || session_failed {
        events.push(SoundEvent::SessionFailure);
    } else if matches!(next.status.as_str(), "stopped" | "idle")
        && previous.is_some_and(|frame| frame.status != next.status && frame.status != "toolFailed")
    {
        events.push(SoundEvent::SessionSuccess);
    }
    events
}

impl NativeSoundObserver {
    pub(crate) fn reset(&mut self) {
        self.frames.clear();
        self.primed_sources.clear();
    }

    pub(crate) fn observe(&mut self, source: &str, snapshot: &Value) {
        let sessions = snapshot
            .get("sessions")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let interactions = snapshot
            .get("interactions")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let primed = self.primed_sources.contains(source);
        let mut next_keys = HashSet::new();

        for session in sessions {
            let Some(session_id) = session.get("id").and_then(Value::as_str) else {
                continue;
            };
            let instance = match source {
                "opencode" | "dsh" => session.get("pluginInstanceId"),
                "pi" => session.get("extensionInstanceId"),
                _ => None,
            }
            .and_then(Value::as_str);
            let key = instance.map_or_else(
                || format!("{source}:{session_id}"),
                |instance| format!("{source}:{instance}:{session_id}"),
            );
            next_keys.insert(key.clone());
            let next = frame_for(source, session, interactions);
            if primed {
                for event in transition_events(self.frames.get(&key), &next) {
                    play(event);
                }
            }
            self.frames.insert(key, next);
        }

        let prefix = format!("{source}:");
        self.frames
            .retain(|key, _| !key.starts_with(&prefix) || next_keys.contains(key));
        self.primed_sources.insert(source.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn transition_events_match_the_frontend_sound_rules() {
        let previous = SoundFrame {
            status: "working".to_string(),
            activity_ids: HashSet::from(["tool-1".to_string()]),
            ..Default::default()
        };
        let next = SoundFrame {
            status: "toolFailed".to_string(),
            activity_ids: HashSet::from(["tool-1".to_string(), "tool-2".to_string()]),
            failed_activity_ids: HashSet::from(["tool-2".to_string()]),
            permission_id: Some("approval-1".to_string()),
            plan_id: Some("plan-1".to_string()),
        };

        assert_eq!(
            transition_events(Some(&previous), &next),
            vec![
                SoundEvent::ToolCall,
                SoundEvent::PermissionApproval,
                SoundEvent::PlanExecution,
                SoundEvent::SessionFailure,
            ]
        );
    }

    #[test]
    fn codex_and_opencode_reviews_map_to_permission_sounds() {
        let codex_session = json!({
            "id": "thread-1",
            "status": "waitingForApproval",
            "activities": []
        });
        let codex_interactions = vec![json!({
            "threadId": "thread-1",
            "requestId": "request-1",
            "kind": "permissionsApproval",
            "resolved": false
        })];
        assert_eq!(
            frame_for("codex", &codex_session, &codex_interactions).permission_id,
            Some("request-1".to_string())
        );

        let opencode_session = json!({
            "id": "session-1",
            "status": "waitingForApproval",
            "activities": [],
            "pendingReviews": [{
                "reviewType": "strictToolGate",
                "reviewId": "review-1"
            }]
        });
        assert_eq!(
            frame_for("opencode", &opencode_session, &[]).permission_id,
            Some("review-1".to_string())
        );
    }

    #[test]
    fn pi_and_dsh_native_reviews_map_to_sound_frames() {
        let pi = json!({
            "id": "session-1",
            "status": "waitingForApproval",
            "activities": [],
            "permission": { "id": "pi-permission" }
        });
        assert_eq!(
            frame_for("pi", &pi, &[]).permission_id,
            Some("pi-permission".to_string())
        );

        let dsh = json!({
            "id": "session-2",
            "status": "waitingForApproval",
            "activities": [],
            "permission": { "id": "dsh-permission" },
            "plan": { "id": "dsh-plan" }
        });
        let frame = frame_for("dsh", &dsh, &[]);
        assert_eq!(frame.permission_id, Some("dsh-permission".to_string()));
        assert_eq!(frame.plan_id, Some("dsh-plan".to_string()));
    }
}
