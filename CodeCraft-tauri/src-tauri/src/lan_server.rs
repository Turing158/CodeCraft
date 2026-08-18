//! LAN web console HTTP server.
//!
//! The console never implements its own approval logic: every write goes
//! through the same functions the desktop panel calls, so decisions stay
//! idempotent and auditable. Reads are served from one merged snapshot that is
//! also pushed over SSE, which keeps browsers from polling.

use std::{
    fs,
    net::{IpAddr, SocketAddr},
    sync::atomic::{AtomicUsize, Ordering},
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    extract::{ConnectInfo, DefaultBodyLimit, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        Html, IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::Manager;
use tokio::sync::oneshot;
use tokio_stream::{wrappers::IntervalStream, StreamExt};

use crate::{
    approval_policy,
    claude_hook::{self, ClaudeQuestionAnswer, PermissionDecision, PlanExecutionMode},
    codex::CodexApprovalDecision,
    lan_auth::{
        cleared_session_cookie, cookie_value, session_cookie, AuthOutcome, LanAuthStore,
        SESSION_COOKIE,
    },
    lan_config::{self, LanBindMode, LanServerConfig},
    lan_net, ApprovalIntegrationState, ClaudeIntegrationState, CodexIntegrationState,
};

/// Custom header a write request must carry. Browsers cannot add it during a
/// cross-site form or image request, so it blocks CSRF even if a cookie leaks.
const REQUEST_HEADER: &str = "x-codecraft-lan";
const MAX_BODY_BYTES: usize = 64 * 1024;
const MAX_SSE_CLIENTS: usize = 8;
const SNAPSHOT_POLL_INTERVAL: Duration = Duration::from_millis(300);
const CONSOLE_HTML: &str = include_str!("../assets/lan/index.html");

const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; \
script-src 'self' 'unsafe-inline'; \
style-src 'self' 'unsafe-inline'; \
img-src 'self' data:; \
connect-src 'self'; \
font-src 'self'; \
base-uri 'none'; \
form-action 'none'; \
frame-ancestors 'none'";

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

#[derive(Debug)]
struct LanRuntime {
    shutdown: Option<oneshot::Sender<()>>,
    address: SocketAddr,
}

#[derive(Default)]
pub(crate) struct LanServerState {
    pub(crate) config: Mutex<LanServerConfig>,
    pub(crate) auth: Mutex<LanAuthStore>,
    runtime: Mutex<Option<LanRuntime>>,
    sse_clients: AtomicUsize,
    last_error: Mutex<Option<String>>,
    last_client_at: Mutex<Option<u64>>,
}

impl LanServerState {
    fn config_snapshot(&self) -> Result<LanServerConfig, String> {
        Ok(self
            .config
            .lock()
            .map_err(|error| error.to_string())?
            .clone())
    }

    fn set_last_error(&self, error: Option<String>) {
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = error;
        }
    }

    fn touch_client(&self) {
        if let Ok(mut slot) = self.last_client_at.lock() {
            *slot = Some(now_ms());
        }
    }
}

/// Address list the settings panel shows, most reachable candidate first.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LanAddress {
    pub address: String,
    pub url: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LanStatus {
    pub running: bool,
    pub port: u16,
    pub bind: LanBindMode,
    pub listen_address: Option<String>,
    pub addresses: Vec<LanAddress>,
    pub client_count: usize,
    pub stream_count: usize,
    pub last_client_at: Option<u64>,
    pub last_error: Option<String>,
    pub allow_approvals: bool,
    pub enabled: bool,
}

pub(crate) fn build_addresses(port: u16, bind: LanBindMode) -> Vec<LanAddress> {
    match bind {
        LanBindMode::Loopback => vec![LanAddress {
            address: "127.0.0.1".to_string(),
            url: format!("http://127.0.0.1:{port}"),
        }],
        LanBindMode::Lan => lan_net::local_lan_addresses()
            .into_iter()
            .map(|address| LanAddress {
                address: address.to_string(),
                url: format!("http://{address}:{port}"),
            })
            .collect(),
    }
}

pub(crate) fn status(state: &LanServerState) -> Result<LanStatus, String> {
    let config = state.config_snapshot()?;
    let runtime = state.runtime.lock().map_err(|error| error.to_string())?;
    let running = runtime.is_some();
    let listen_address = runtime.as_ref().map(|runtime| runtime.address.to_string());
    drop(runtime);

    Ok(LanStatus {
        running,
        port: config.port,
        bind: config.bind,
        listen_address,
        addresses: if running {
            build_addresses(config.port, config.bind)
        } else {
            Vec::new()
        },
        client_count: state
            .auth
            .lock()
            .map_err(|error| error.to_string())?
            .session_count(),
        stream_count: state.sse_clients.load(Ordering::Relaxed),
        last_client_at: *state
            .last_client_at
            .lock()
            .map_err(|error| error.to_string())?,
        last_error: state
            .last_error
            .lock()
            .map_err(|error| error.to_string())?
            .clone(),
        allow_approvals: config.allow_approvals,
        enabled: config.enabled,
    })
}

/// Appends one line describing a remote decision. The desktop panel writes no
/// audit entry, so the source=lan marker is enough to tell them apart later.
fn append_remote_audit(action: &str, request_id: &str, decision: &str, address: IpAddr) {
    let directory = approval_policy::base_data_dir();
    if fs::create_dir_all(&directory).is_err() {
        return;
    }
    let line = format!(
        "{} source=lan action={} request={} decision={} client={}\n",
        now_ms(),
        action,
        request_id,
        decision,
        address
    );
    if let Ok(mut file) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("lan-audit.log"))
    {
        use std::io::Write;
        let _ = file.write_all(line.as_bytes());
    }
}

#[derive(Clone)]
struct LanHttpState {
    app: tauri::AppHandle,
}

/// Reads the current Claude and Codex state plus the flags the console needs.
fn snapshot_value(app: &tauri::AppHandle) -> Value {
    let claude = crate::claude_snapshot(&app.state::<ClaudeIntegrationState>())
        .ok()
        .and_then(|snapshot| serde_json::to_value(snapshot).ok())
        .unwrap_or(Value::Null);
    let codex = crate::codex_snapshot(&app.state::<CodexIntegrationState>())
        .ok()
        .and_then(|snapshot| serde_json::to_value(snapshot).ok())
        .unwrap_or(Value::Null);
    let approval_mode = app
        .state::<ApprovalIntegrationState>()
        .settings
        .lock()
        .ok()
        .map(|settings| settings.mode)
        .and_then(|mode| serde_json::to_value(mode).ok())
        .unwrap_or(Value::Null);
    let config = app.state::<LanServerState>().config_snapshot().ok();

    json!({
        "generatedAt": now_ms(),
        "allowApprovals": config
            .as_ref()
            .map(|config| config.allow_approvals)
            .unwrap_or(false),
        "approvalMode": approval_mode,
        "claude": claude,
        "codex": codex,
    })
}

/// FNV-1a over the serialized snapshot so an SSE stream only pushes on change.
fn snapshot_fingerprint(snapshot: &str) -> u64 {
    snapshot.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

fn json_error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

enum Access {
    Granted,
    Denied(Response),
}

/// Requires a live session cookie. Write requests also need the custom header
/// and the remote-approval switch turned on.
fn check_access(
    state: &LanServerState,
    headers: &HeaderMap,
    writing: bool,
) -> Result<Access, String> {
    let session = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| cookie_value(value, SESSION_COOKIE));
    let authorized = session
        .as_deref()
        .map(|session| {
            state
                .auth
                .lock()
                .map(|auth| auth.session_is_valid(session))
                .unwrap_or(false)
        })
        .unwrap_or(false);
    if !authorized {
        return Ok(Access::Denied(
            (
                StatusCode::UNAUTHORIZED,
                [(header::SET_COOKIE, cleared_session_cookie())],
                Json(json!({ "error": "需要重新登录" })),
            )
                .into_response(),
        ));
    }

    if writing {
        if !headers.contains_key(REQUEST_HEADER) {
            return Ok(Access::Denied(json_error(
                StatusCode::BAD_REQUEST,
                "缺少 CodeCraft 请求头",
            )));
        }
        if !state.config_snapshot()?.allow_approvals {
            return Ok(Access::Denied(json_error(
                StatusCode::FORBIDDEN,
                "远程审批已关闭，网页当前为只读",
            )));
        }
    }

    Ok(Access::Granted)
}

async fn serve_console() -> impl IntoResponse {
    Html(CONSOLE_HTML)
}

#[derive(Deserialize)]
struct AuthRequest {
    token: String,
}

async fn post_auth(
    State(http_state): State<LanHttpState>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    Json(request): Json<AuthRequest>,
) -> Response {
    let state = http_state.app.state::<LanServerState>();
    let address = client.ip();

    let locked_out = state
        .auth
        .lock()
        .map(|auth| auth.is_locked_out(address))
        .unwrap_or(false);
    if locked_out {
        return json_error(StatusCode::TOO_MANY_REQUESTS, "尝试次数过多，请稍后再试");
    }

    let Ok(config) = state.config_snapshot() else {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "无法读取服务配置");
    };
    if !lan_config::tokens_match(&config.token, request.token.trim()) {
        let outcome = state
            .auth
            .lock()
            .map(|mut auth| auth.record_failure(address))
            .unwrap_or(AuthOutcome::Rejected);
        return match outcome {
            AuthOutcome::LockedOut => {
                json_error(StatusCode::TOO_MANY_REQUESTS, "尝试次数过多，请稍后再试")
            }
            _ => json_error(StatusCode::UNAUTHORIZED, "访问令牌不正确"),
        };
    }

    let Ok(session) = state.auth.lock().map(|mut auth| auth.grant_session(address)) else {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "无法创建会话");
    };
    state.touch_client();

    (
        StatusCode::OK,
        [(header::SET_COOKIE, session_cookie(&session))],
        Json(json!({ "allowApprovals": config.allow_approvals })),
    )
        .into_response()
}

async fn post_logout() -> Response {
    (
        StatusCode::OK,
        [(header::SET_COOKIE, cleared_session_cookie())],
        Json(json!({ "ok": true })),
    )
        .into_response()
}

async fn get_state(State(http_state): State<LanHttpState>, headers: HeaderMap) -> Response {
    let state = http_state.app.state::<LanServerState>();
    match check_access(&state, &headers, false) {
        Ok(Access::Denied(response)) => return response,
        Err(error) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &error),
        Ok(Access::Granted) => {}
    }
    state.touch_client();
    Json(snapshot_value(&http_state.app)).into_response()
}

async fn get_events(State(http_state): State<LanHttpState>, headers: HeaderMap) -> Response {
    let state = http_state.app.state::<LanServerState>();
    match check_access(&state, &headers, false) {
        Ok(Access::Denied(response)) => return response,
        Err(error) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &error),
        Ok(Access::Granted) => {}
    }

    if state.sse_clients.load(Ordering::Relaxed) >= MAX_SSE_CLIENTS {
        return json_error(StatusCode::SERVICE_UNAVAILABLE, "实时连接数已达上限");
    }
    state.sse_clients.fetch_add(1, Ordering::Relaxed);
    state.touch_client();

    let app = http_state.app.clone();

    // Releases the slot when the browser disconnects, including on page reload.
    struct StreamGuard {
        app: tauri::AppHandle,
    }
    impl Drop for StreamGuard {
        fn drop(&mut self) {
            self.app
                .state::<LanServerState>()
                .sse_clients
                .fetch_sub(1, Ordering::Relaxed);
        }
    }
    let guard = StreamGuard { app: app.clone() };

    let mut last_fingerprint: Option<u64> = None;
    let stream =
        IntervalStream::new(tokio::time::interval(SNAPSHOT_POLL_INTERVAL)).filter_map(move |_| {
            let _guard = &guard;
            let snapshot = snapshot_value(&app).to_string();
            let fingerprint = snapshot_fingerprint(&snapshot);
            if last_fingerprint == Some(fingerprint) {
                return None;
            }
            last_fingerprint = Some(fingerprint);
            Some(Ok::<Event, std::convert::Infallible>(
                Event::default().event("state").data(snapshot),
            ))
        });

    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudePermissionBody {
    request_id: String,
    decision: PermissionDecision,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeQuestionBody {
    request_id: String,
    answers: Vec<ClaudeQuestionAnswer>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudePlanBody {
    request_id: String,
    note: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexApprovalBody {
    request_id: String,
    decision: CodexApprovalDecision,
}

/// Runs a write handler after the shared access checks, then records the audit
/// line when the config asks for it.
fn with_write_access<F>(
    http_state: &LanHttpState,
    headers: &HeaderMap,
    client: SocketAddr,
    action: &str,
    request_id: &str,
    decision: &str,
    apply: F,
) -> Response
where
    F: FnOnce() -> Result<(), String>,
{
    let state = http_state.app.state::<LanServerState>();
    match check_access(&state, headers, true) {
        Ok(Access::Denied(response)) => return response,
        Err(error) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &error),
        Ok(Access::Granted) => {}
    }
    state.touch_client();

    match apply() {
        Ok(()) => {
            if state
                .config_snapshot()
                .map(|config| config.audit_remote)
                .unwrap_or(true)
            {
                append_remote_audit(action, request_id, decision, client.ip());
            }
            Json(json!({ "ok": true })).into_response()
        }
        Err(error) => json_error(StatusCode::BAD_REQUEST, &error),
    }
}

async fn post_claude_permission(
    State(http_state): State<LanHttpState>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<ClaudePermissionBody>,
) -> Response {
    let app = http_state.app.clone();
    let decision_label = match body.decision {
        PermissionDecision::Allow => "allow",
        PermissionDecision::AllowAlways => "allowAlways",
        PermissionDecision::Deny => "deny",
    };
    with_write_access(
        &http_state,
        &headers,
        client,
        "claude-permission",
        &body.request_id,
        decision_label,
        || {
            crate::apply_claude_permission_decision(
                &app.state::<ClaudeIntegrationState>(),
                &body.request_id,
                body.decision,
            )
        },
    )
}

async fn post_claude_question(
    State(http_state): State<LanHttpState>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<ClaudeQuestionBody>,
) -> Response {
    with_write_access(
        &http_state,
        &headers,
        client,
        "claude-question",
        &body.request_id,
        "answer",
        || claude_hook::submit_question_answer(&body.request_id, &body.answers),
    )
}

async fn post_claude_plan(
    State(http_state): State<LanHttpState>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<ClaudePlanBody>,
) -> Response {
    let app = http_state.app.clone();
    with_write_access(
        &http_state,
        &headers,
        client,
        "claude-plan",
        &body.request_id,
        "auto",
        || {
            crate::apply_claude_plan_decision(
                &app.state::<ClaudeIntegrationState>(),
                &body.request_id,
                PlanExecutionMode::Auto,
                body.note.clone(),
            )
        },
    )
}

async fn post_codex_approval(
    State(http_state): State<LanHttpState>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<CodexApprovalBody>,
) -> Response {
    let app = http_state.app.clone();
    let decision_label = match body.decision {
        CodexApprovalDecision::Accept => "accept",
        CodexApprovalDecision::AcceptForSession => "acceptForSession",
        CodexApprovalDecision::Decline => "decline",
        CodexApprovalDecision::Cancel => "cancel",
    };
    with_write_access(
        &http_state,
        &headers,
        client,
        "codex-approval",
        &body.request_id,
        decision_label,
        || {
            crate::apply_codex_approval(
                &app.state::<CodexIntegrationState>(),
                &body.request_id,
                body.decision,
            )
        },
    )
}

/// Adds the headers that keep session data out of caches and stop the page from
/// being framed or sniffed.
async fn security_headers(request: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, must-revalidate"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    );
    response
}

fn router(app: tauri::AppHandle) -> Router {
    Router::new()
        .route("/", get(serve_console))
        .route("/api/auth", post(post_auth))
        .route("/api/logout", post(post_logout))
        .route("/api/state", get(get_state))
        .route("/api/events", get(get_events))
        .route("/api/claude/permission", post(post_claude_permission))
        .route("/api/claude/question", post(post_claude_question))
        .route("/api/claude/plan", post(post_claude_plan))
        .route("/api/codex/approval", post(post_codex_approval))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(axum::middleware::from_fn(security_headers))
        .with_state(LanHttpState { app })
}

/// Binds synchronously so a busy port surfaces in the settings panel right away
/// instead of failing silently inside a background task.
pub(crate) fn start(app: &tauri::AppHandle) -> Result<LanStatus, String> {
    let state = app.state::<LanServerState>();
    let config = state.config_snapshot()?;
    lan_config::validate_port(config.port)?;

    if state
        .runtime
        .lock()
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return status(&state);
    }

    let bind_address = SocketAddr::from((config.bind.bind_ip(), config.port));
    let listener =
        tauri::async_runtime::block_on(
            async move { tokio::net::TcpListener::bind(bind_address).await },
        )
        .map_err(|error| format!("无法监听 {bind_address}：{error}"))?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;

    let (shutdown_sender, shutdown_receiver) = oneshot::channel::<()>();
    let service = router(app.clone());
    tauri::async_runtime::spawn(async move {
        let _ = axum::serve(
            listener,
            service.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let _ = shutdown_receiver.await;
        })
        .await;
    });

    *state.runtime.lock().map_err(|error| error.to_string())? = Some(LanRuntime {
        shutdown: Some(shutdown_sender),
        address,
    });
    state.set_last_error(None);
    status(&state)
}

/// Signals graceful shutdown and drops every session so the port and the
/// existing logins stop being usable immediately.
pub(crate) fn stop(app: &tauri::AppHandle) -> Result<LanStatus, String> {
    let state = app.state::<LanServerState>();
    if let Some(mut runtime) = state
        .runtime
        .lock()
        .map_err(|error| error.to_string())?
        .take()
    {
        if let Some(shutdown) = runtime.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
    state
        .auth
        .lock()
        .map_err(|error| error.to_string())?
        .invalidate_all();
    status(&state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> IpAddr {
        IpAddr::from([192, 168, 1, 40])
    }

    /// Builds a state with one live session and returns its cookie header.
    fn state_with_session(allow_approvals: bool) -> (LanServerState, HeaderMap) {
        let state = LanServerState::default();
        {
            let mut config = state.config.lock().expect("config lock");
            config.allow_approvals = allow_approvals;
        }
        let session = state
            .auth
            .lock()
            .expect("auth lock")
            .grant_session(client());

        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("{SESSION_COOKIE}={session}")).expect("cookie"),
        );
        (state, headers)
    }

    fn denied_status(access: Result<Access, String>) -> Option<StatusCode> {
        match access.expect("access check") {
            Access::Denied(response) => Some(response.status()),
            Access::Granted => None,
        }
    }

    #[test]
    fn identical_snapshots_share_a_fingerprint() {
        let first = snapshot_fingerprint("{\"a\":1}");
        let second = snapshot_fingerprint("{\"a\":1}");
        let third = snapshot_fingerprint("{\"a\":2}");

        assert_eq!(first, second);
        assert_ne!(first, third);
    }

    #[test]
    fn loopback_mode_only_advertises_the_local_address() {
        let addresses = build_addresses(8787, LanBindMode::Loopback);

        assert_eq!(addresses.len(), 1);
        assert_eq!(addresses[0].url, "http://127.0.0.1:8787");
    }

    #[test]
    fn the_console_page_is_embedded_in_the_binary() {
        let markup = CONSOLE_HTML.to_ascii_lowercase();
        assert!(markup.contains("<html") || markup.contains("<!doctype"));
    }

    #[test]
    fn the_policy_blocks_framing_and_third_party_requests() {
        assert!(CONTENT_SECURITY_POLICY.contains("frame-ancestors 'none'"));
        assert!(CONTENT_SECURITY_POLICY.contains("default-src 'self'"));
    }

    #[test]
    fn a_request_without_a_session_cookie_is_rejected() {
        let state = LanServerState::default();

        assert_eq!(
            denied_status(check_access(&state, &HeaderMap::new(), false)),
            Some(StatusCode::UNAUTHORIZED),
        );
    }

    #[test]
    fn a_live_session_may_read_the_snapshot() {
        let (state, headers) = state_with_session(false);

        assert_eq!(denied_status(check_access(&state, &headers, false)), None);
    }

    #[test]
    fn writes_need_the_custom_header_even_with_a_valid_cookie() {
        let (state, headers) = state_with_session(true);

        assert_eq!(
            denied_status(check_access(&state, &headers, true)),
            Some(StatusCode::BAD_REQUEST),
        );
    }

    #[test]
    fn read_only_mode_refuses_every_write() {
        let (state, mut headers) = state_with_session(false);
        headers.insert(REQUEST_HEADER, HeaderValue::from_static("1"));

        assert_eq!(
            denied_status(check_access(&state, &headers, true)),
            Some(StatusCode::FORBIDDEN),
        );
    }

    #[test]
    fn remote_approvals_allow_a_write_once_both_checks_pass() {
        let (state, mut headers) = state_with_session(true);
        headers.insert(REQUEST_HEADER, HeaderValue::from_static("1"));

        assert_eq!(denied_status(check_access(&state, &headers, true)), None);
    }

    #[test]
    fn rotating_the_token_locks_the_browser_out_again() {
        let (state, headers) = state_with_session(true);
        state
            .auth
            .lock()
            .expect("auth lock")
            .invalidate_all();

        assert_eq!(
            denied_status(check_access(&state, &headers, false)),
            Some(StatusCode::UNAUTHORIZED),
        );
    }

    #[test]
    fn a_stopped_server_reports_no_addresses_and_no_clients() {
        let state = LanServerState::default();

        let status = status(&state).expect("status");

        assert!(!status.running);
        assert!(status.addresses.is_empty());
        assert_eq!(status.client_count, 0);
        assert_eq!(status.stream_count, 0);
        assert!(!status.allow_approvals);
    }

    #[test]
    fn a_running_status_counts_the_logged_in_browsers() {
        let (state, _headers) = state_with_session(true);

        let status = status(&state).expect("status");

        assert_eq!(status.client_count, 1);
        assert!(status.allow_approvals);
    }
}
