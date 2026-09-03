use std::{
    collections::HashMap,
    env,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem},
    tray::TrayIconBuilder,
    Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, WebviewWindow,
    WebviewWindowBuilder,
};

mod approval_policy;
mod claude_hook;
mod codex;
mod codex_hook;
mod dsh;
mod dsh_hook;
mod hook_config;
mod lan_auth;
mod lan_config;
mod lan_net;
mod lan_server;
mod native_sound;
mod opencode;
mod opencode_hook;
mod mimo;
mod mimo_hook;
mod pi;
mod pi_hook;
mod zcode;
mod zcode_hook;

pub use claude_hook::capture_claude_hook;
pub use codex_hook::capture_codex_hook;
pub use zcode_hook::capture_zcode_hook;

const PANEL_WIDTH: f64 = 500.0;
const MIN_PANEL_WIDTH: f64 = 360.0;
const MAX_PANEL_WIDTH: f64 = 820.0;
const COLLAPSED_HEIGHT: f64 = 5.0;
const MIN_PANEL_HEIGHT: f64 = 3.0;
const MAX_PANEL_HEIGHT: f64 = 1200.0;
const GROW_DURATION: Duration = Duration::from_millis(180);
const SHRINK_DURATION: Duration = Duration::from_millis(140);
const POSITION_DURATION: Duration = Duration::from_millis(180);
const ANIMATION_FRAME_INTERVAL: Duration = Duration::from_millis(16);
const PANEL_INSET: f64 = 12.0;
// Keep the native window region aligned with the webview's visible bottom corners.
const MIN_VISIBLE_BOTTOM_CORNER_RADIUS: f64 = 12.0;
const TRAY_ICON_ID: &str = "codecraft-tray";
const REOPEN_REQUESTED_EVENT: &str = "reopen-requested";
const OPEN_SETTINGS_REQUESTED_EVENT: &str = "open-settings-requested";
const APPROVAL_SETTINGS_CHANGED_EVENT: &str = "approval-settings-changed";

static PANEL_ANIMATION_GENERATION: AtomicU64 = AtomicU64::new(0);
static PANEL_POSITION_ANIMATION_GENERATION: AtomicU64 = AtomicU64::new(0);
static REOPEN_REQUESTED: AtomicBool = AtomicBool::new(false);
static OPEN_SETTINGS_REQUESTED: AtomicBool = AtomicBool::new(false);
static PANEL_RESIZE_LOCK: Mutex<()> = Mutex::new(());
static PANEL_POSITION_LOCK: Mutex<()> = Mutex::new(());
// Hook status reads can repair managed files, so they must not race with an
// install or uninstall. Keep this lock separate from the panel locks so the
// slow refresh cannot stall unrelated window and settings commands.
static HOOK_CONFIGURATION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn hook_configuration_lock() -> Result<std::sync::MutexGuard<'static, ()>, String> {
    HOOK_CONFIGURATION_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|error| error.to_string())
}

#[derive(Default)]
struct ClaudeIntegrationState {
    sessions: Mutex<claude_hook::ClaudeSessionStore>,
    hook_error: Mutex<Option<String>>,
}

#[derive(Default)]
struct ApprovalIntegrationState {
    settings: Mutex<approval_policy::ApprovalSettings>,
}

#[derive(Default)]
struct NativeSoundIntegrationState {
    settings: Mutex<native_sound::NativeSoundSettings>,
}

#[derive(Default)]
struct TrayMenuState {
    active_session_count: AtomicUsize,
}

#[derive(Default)]
struct CodexIntegrationState {
    hook_config: Mutex<codex_hook::CodexHookConfig>,
    hook_store: Mutex<codex::CodexStore>,
    hook_error: Mutex<Option<String>>,
}

#[derive(Default)]
struct OpenCodeIntegrationState {
    store: Mutex<opencode::OpenCodeStore>,
    hook_error: Mutex<Option<String>>,
}

#[derive(Default)]
struct MimoIntegrationState {
    store: Mutex<mimo::MimoStore>,
    hook_error: Mutex<Option<String>>,
}

#[derive(Default)]
struct PiIntegrationState {
    store: Mutex<pi::PiStore>,
    hook_error: Mutex<Option<String>>,
}

#[derive(Default)]
struct DshIntegrationState {
    store: Mutex<dsh::DshStore>,
    pending: Mutex<HashMap<String, tokio::sync::oneshot::Sender<serde_json::Value>>>,
    hook_error: Mutex<Option<String>>,
    bridge: Mutex<Option<dsh_hook::DshBridgeInfo>>,
}

#[derive(Default)]
struct ZCodeIntegrationState {
    store: Mutex<zcode::ZCodeStore>,
    hook_error: Mutex<Option<String>>,
}

struct PanelWindowState {
    horizontal_position: Mutex<f64>,
}

impl Default for PanelWindowState {
    fn default() -> Self {
        Self {
            horizontal_position: Mutex::new(0.5),
        }
    }
}

#[derive(Clone, Copy)]
struct PanelShapeRequest {
    panel_width: f64,
    interface_scale: f64,
    content_height: f64,
    collapsed_height: f64,
    collapsed_corner_progress: f64,
}

#[derive(Default)]
struct PanelShapeState {
    request: Mutex<Option<PanelShapeRequest>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeSessionSnapshot {
    pub(crate) connected: bool,
    pub(crate) integration_error: Option<String>,
    pub(crate) sessions: Vec<claude_hook::ClaudeSession>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum HookAgentId {
    ClaudeCode,
    Codex,
    OpenCode,
    Mimo,
    Pi,
    DeepSeekHarness,
    ZCode,
}

impl hook_config::HookInstallConfig {
    fn is_enabled(&self, agent: HookAgentId) -> bool {
        match agent {
            HookAgentId::ClaudeCode => self.claude_code,
            HookAgentId::Codex => self.codex,
            HookAgentId::OpenCode => self.open_code,
            HookAgentId::Mimo => self.mimo,
            HookAgentId::Pi => self.pi,
            HookAgentId::DeepSeekHarness => self.deep_seek_harness,
            HookAgentId::ZCode => self.z_code,
        }
    }

    fn set_enabled(&mut self, agent: HookAgentId, enabled: bool) {
        match agent {
            HookAgentId::ClaudeCode => self.claude_code = enabled,
            HookAgentId::Codex => self.codex = enabled,
            HookAgentId::OpenCode => self.open_code = enabled,
            HookAgentId::Mimo => self.mimo = enabled,
            HookAgentId::Pi => self.pi = enabled,
            HookAgentId::DeepSeekHarness => self.deep_seek_harness = enabled,
            HookAgentId::ZCode => self.z_code = enabled,
        }
    }
}

impl HookAgentId {
    fn command(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::Mimo => "mimo",
            Self::Pi => "pi",
            Self::DeepSeekHarness => "dsh",
            Self::ZCode => "zcode",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
            Self::OpenCode => "OpenCode",
            Self::Mimo => "Mimo",
            Self::Pi => "PI",
            Self::DeepSeekHarness => "DeepSeek Harness",
            Self::ZCode => "ZCode",
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HookIntegrationStatus {
    id: HookAgentId,
    name: &'static str,
    agent_installed: bool,
    hook_installed: bool,
    install_state: Option<String>,
    install_path: Option<String>,
    bundled_version: Option<String>,
    installed_version: Option<String>,
    running_versions: Vec<String>,
    error: Option<String>,
}

fn command_is_installed(command: &str) -> bool {
    if Path::new(command).is_file() {
        return true;
    }
    let Some(path_value) = env::var_os("PATH") else {
        return false;
    };

    #[cfg(windows)]
    let extensions: Vec<String> = env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .filter(|extension| !extension.is_empty())
        .map(|extension| extension.to_ascii_lowercase())
        .collect();

    env::split_paths(&path_value).any(|directory| {
        if directory.join(command).is_file() {
            return true;
        }
        #[cfg(windows)]
        {
            return extensions
                .iter()
                .any(|extension| directory.join(format!("{command}{extension}")).is_file());
        }
        #[cfg(not(windows))]
        false
    })
}

fn agent_is_installed(agent: HookAgentId, state: &CodexIntegrationState) -> Result<bool, String> {
    if matches!(agent, HookAgentId::Pi) {
        return Ok(command_is_installed(agent.command()) || command_is_installed("pi.ps1"));
    }
    if matches!(agent, HookAgentId::DeepSeekHarness) {
        return Ok(command_is_installed("dsh")
            || command_is_installed("dsh.cmd")
            || command_is_installed("npx")
            || command_is_installed("npx.cmd"));
    }
    if matches!(agent, HookAgentId::ZCode) {
        return Ok(zcode_hook::detect_zcode_installation().is_some());
    }
    let _ = state;
    Ok(command_is_installed(agent.command()))
}

fn opencode_install_state_label(state: opencode_hook::OpenCodeHookInstallState) -> String {
    match state {
        opencode_hook::OpenCodeHookInstallState::NotInstalled => "notInstalled",
        opencode_hook::OpenCodeHookInstallState::Installed => "installed",
        opencode_hook::OpenCodeHookInstallState::SyncedRestartRequired => "syncedRestartRequired",
        opencode_hook::OpenCodeHookInstallState::Modified => "modified",
        opencode_hook::OpenCodeHookInstallState::Conflict => "conflict",
        opencode_hook::OpenCodeHookInstallState::Incompatible => "incompatible",
        opencode_hook::OpenCodeHookInstallState::Error => "error",
    }
    .to_string()
}

fn mimo_install_state_label(state: mimo_hook::MimoHookInstallState) -> String {
    match state {
        mimo_hook::MimoHookInstallState::NotInstalled => "notInstalled",
        mimo_hook::MimoHookInstallState::Installed => "installed",
        mimo_hook::MimoHookInstallState::SyncedRestartRequired => "syncedRestartRequired",
        mimo_hook::MimoHookInstallState::Modified => "modified",
        mimo_hook::MimoHookInstallState::Conflict => "conflict",
        mimo_hook::MimoHookInstallState::Incompatible => "incompatible",
        mimo_hook::MimoHookInstallState::Error => "error",
    }
    .to_string()
}

fn pi_install_state_label(state: pi_hook::PiHookInstallState) -> String {
    match state {
        pi_hook::PiHookInstallState::NotInstalled => "notInstalled",
        pi_hook::PiHookInstallState::Installed => "installed",
        pi_hook::PiHookInstallState::Modified => "modified",
        pi_hook::PiHookInstallState::Conflict => "conflict",
        pi_hook::PiHookInstallState::Incompatible => "incompatible",
        pi_hook::PiHookInstallState::Error => "error",
    }
    .to_string()
}

fn dsh_install_state_label(state: dsh_hook::DshHookInstallState) -> String {
    match state {
        dsh_hook::DshHookInstallState::NotInstalled => "notInstalled",
        dsh_hook::DshHookInstallState::Installed => "installed",
        dsh_hook::DshHookInstallState::Modified => "modified",
        dsh_hook::DshHookInstallState::Conflict => "conflict",
        dsh_hook::DshHookInstallState::Incompatible => "incompatible",
    }
    .to_string()
}

fn zcode_install_state_label(state: zcode_hook::ZCodeHookInstallState) -> String {
    match state {
        zcode_hook::ZCodeHookInstallState::NotInstalled => "notInstalled",
        zcode_hook::ZCodeHookInstallState::Installed => "installed",
        zcode_hook::ZCodeHookInstallState::Modified => "modified",
        zcode_hook::ZCodeHookInstallState::Conflict => "conflict",
        zcode_hook::ZCodeHookInstallState::Incompatible => "incompatible",
        zcode_hook::ZCodeHookInstallState::Error => "error",
    }
    .to_string()
}

fn codex_hook_project_dir(state: &CodexIntegrationState) -> Result<Option<String>, String> {
    Ok(state
        .hook_config
        .lock()
        .map_err(|error| error.to_string())?
        .project_dir
        .clone())
}

fn install_codex_hooks(executable: &Path, project_dir: Option<&Path>) -> Result<String, String> {
    codex_hook::install_codex_hooks(executable, project_dir)
}

fn save_hook_installation_state(agent: HookAgentId, enabled: bool) -> Result<(), String> {
    let mut config = hook_config::load();
    config.set_enabled(agent, enabled);
    hook_config::save(&config)
}

pub(crate) fn hook_statuses(
    state: &CodexIntegrationState,
    opencode_state: &OpenCodeIntegrationState,
    mimo_state: &MimoIntegrationState,
    dsh_state: &DshIntegrationState,
    zcode_state: &ZCodeIntegrationState,
) -> Result<Vec<HookIntegrationStatus>, String> {
    let _lock = hook_configuration_lock()?;
    hook_statuses_unlocked(state, opencode_state, mimo_state, dsh_state, zcode_state)
}

fn hook_statuses_unlocked(
    state: &CodexIntegrationState,
    opencode_state: &OpenCodeIntegrationState,
    mimo_state: &MimoIntegrationState,
    dsh_state: &DshIntegrationState,
    zcode_state: &ZCodeIntegrationState,
) -> Result<Vec<HookIntegrationStatus>, String> {
    let project_dir = codex_hook_project_dir(state)?;
    let opencode = opencode_hook::status_and_sync()?;
    *opencode_state
        .hook_error
        .lock()
        .map_err(|error| error.to_string())? = opencode.error.clone();
    Ok(vec![
        HookIntegrationStatus {
            id: HookAgentId::ClaudeCode,
            name: HookAgentId::ClaudeCode.display_name(),
            agent_installed: agent_is_installed(HookAgentId::ClaudeCode, state)?,
            hook_installed: claude_hook::claude_hooks_installed()?,
            install_state: None,
            install_path: None,
            bundled_version: None,
            installed_version: None,
            running_versions: Vec::new(),
            error: None,
        },
        HookIntegrationStatus {
            id: HookAgentId::Codex,
            name: HookAgentId::Codex.display_name(),
            agent_installed: agent_is_installed(HookAgentId::Codex, state)?,
            hook_installed: codex_hook::codex_hooks_installed(
                project_dir.as_deref().map(Path::new),
            )?,
            install_state: None,
            install_path: None,
            bundled_version: None,
            installed_version: None,
            running_versions: Vec::new(),
            error: None,
        },
        HookIntegrationStatus {
            id: HookAgentId::OpenCode,
            name: HookAgentId::OpenCode.display_name(),
            agent_installed: agent_is_installed(HookAgentId::OpenCode, state)?,
            hook_installed: opencode.installed(),
            install_state: Some(opencode_install_state_label(opencode.state)),
            install_path: Some(opencode.install_path),
            bundled_version: Some(opencode.bundled_version.to_string()),
            installed_version: opencode.installed_version,
            running_versions: opencode.running_versions,
            error: opencode.error,
        },
        {
            let status = mimo_hook::status()?;
            let runtime_error = mimo_state
                .hook_error
                .lock()
                .map_err(|error| error.to_string())?
                .clone();
            HookIntegrationStatus {
                id: HookAgentId::Mimo,
                name: HookAgentId::Mimo.display_name(),
                agent_installed: agent_is_installed(HookAgentId::Mimo, state)?,
                hook_installed: status.installed(),
                install_state: Some(mimo_install_state_label(status.state)),
                install_path: Some(status.install_path),
                bundled_version: Some(status.bundled_version.to_string()),
                installed_version: status.installed_version,
                running_versions: status.running_versions,
                error: status.error.or(runtime_error),
            }
        },
        {
            let status = pi_hook::status()?;
            HookIntegrationStatus {
                id: HookAgentId::Pi,
                name: HookAgentId::Pi.display_name(),
                agent_installed: agent_is_installed(HookAgentId::Pi, state)?,
                hook_installed: status.installed(),
                install_state: Some(pi_install_state_label(status.state)),
                install_path: Some(status.install_path),
                bundled_version: Some(status.bundled_version.to_string()),
                installed_version: status.installed_version,
                running_versions: Vec::new(),
                error: status.error,
            }
        },
        {
            let status = dsh_hook::status()?;
            let running_versions = dsh_state
                .store
                .lock()
                .map_err(|error| error.to_string())?
                .snapshot()
                .instances
                .into_iter()
                .map(|instance| instance.dsh_version)
                .collect();
            let runtime_error = dsh_state
                .hook_error
                .lock()
                .map_err(|error| error.to_string())?
                .clone();
            HookIntegrationStatus {
                id: HookAgentId::DeepSeekHarness,
                name: HookAgentId::DeepSeekHarness.display_name(),
                agent_installed: agent_is_installed(HookAgentId::DeepSeekHarness, state)?,
                hook_installed: status.installed(),
                install_state: Some(dsh_install_state_label(status.state)),
                install_path: Some(status.install_path),
                bundled_version: Some(status.bundled_version.to_string()),
                installed_version: status.installed_version,
                running_versions,
                error: status.error.or(runtime_error),
            }
        },
        {
            let status = zcode_hook::status()?;
            let runtime_error = zcode_state
                .hook_error
                .lock()
                .map_err(|error| error.to_string())?
                .clone();
            HookIntegrationStatus {
                id: HookAgentId::ZCode,
                name: HookAgentId::ZCode.display_name(),
                agent_installed: agent_is_installed(HookAgentId::ZCode, state)?,
                hook_installed: status.installed(),
                install_state: Some(zcode_install_state_label(status.state)),
                install_path: Some(status.install_path),
                bundled_version: Some("3.10.1".to_string()),
                installed_version: status.detected_version,
                running_versions: Vec::new(),
                error: status.error.or(runtime_error),
            }
        },
    ])
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct MonitorSignature {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale_factor: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PanelShapeGeometry {
    outer_edge: f64,
    outer_control: f64,
    shoulder_control_y: f64,
    shoulder_y: f64,
    corner_radius: f64,
    lower_corner_y: f64,
    lower_inner_edge: f64,
    lower_outer_edge: f64,
    right_outer_control: f64,
    right_outer_edge: f64,
    bottom: f64,
}

fn interpolate(start: f64, end: f64, progress: f64) -> f64 {
    start + (end - start) * progress
}

fn normalized_corner_progress(value: Option<f64>) -> f64 {
    value
        .filter(|value| value.is_finite())
        .unwrap_or(0.0)
        .clamp(0.0, 1.0)
}

fn collapsed_strip_shape(
    panel_width: f64,
    interface_scale: f64,
    collapsed_height: f64,
) -> PanelShapeGeometry {
    let panel_inset = PANEL_INSET * interface_scale;
    let panel_right_inset = panel_width - panel_inset;
    let top_corner_radius = 1.75 * interface_scale;

    PanelShapeGeometry {
        outer_edge: panel_inset - top_corner_radius,
        outer_control: panel_inset - top_corner_radius / 2.0,
        shoulder_control_y: top_corner_radius * 2.0 / 3.0,
        shoulder_y: top_corner_radius,
        corner_radius: 0.0,
        lower_corner_y: collapsed_height,
        lower_inner_edge: panel_inset,
        lower_outer_edge: panel_right_inset,
        right_outer_control: panel_right_inset + top_corner_radius / 2.0,
        right_outer_edge: panel_right_inset + top_corner_radius,
        bottom: collapsed_height,
    }
}

fn panel_shape_geometry(
    panel_width: f64,
    interface_scale: f64,
    window_height: f64,
    content_height: f64,
    collapsed_height: f64,
    collapsed_corner_progress: f64,
) -> PanelShapeGeometry {
    let panel_inset = PANEL_INSET * interface_scale;
    let panel_right_inset = panel_width - panel_inset;
    let expanded_height = content_height.max(collapsed_height);
    let height = window_height.clamp(collapsed_height, expanded_height);
    if collapsed_corner_progress == 0.0 && height <= collapsed_height + 1e-9 {
        return collapsed_strip_shape(panel_width, interface_scale, collapsed_height);
    }
    let height_progress = if expanded_height == collapsed_height {
        0.0
    } else {
        (height - collapsed_height) / (expanded_height - collapsed_height)
    };
    let corner_progress = interpolate(collapsed_corner_progress, 1.0, height_progress);
    // Keep the lower curve below the upper shoulder. GDI's PathToRegion can
    // flatten a self-intersecting short path into a square-ended region.
    let requested_corner_radius = (interpolate(4.0, 6.0, corner_progress) * interface_scale)
        .max(MIN_VISIBLE_BOTTOM_CORNER_RADIUS * corner_progress);
    let shoulder_y = interpolate(0.0, 9.0 * interface_scale, corner_progress);
    let max_width_corner_radius = ((panel_right_inset - panel_inset) / 2.0).max(0.0);
    let corner_radius = requested_corner_radius
        .min((height - shoulder_y).max(0.0))
        .min(max_width_corner_radius);

    PanelShapeGeometry {
        outer_edge: interpolate(panel_inset, 0.0, corner_progress),
        outer_control: interpolate(panel_inset, 7.0 * interface_scale, corner_progress),
        shoulder_control_y: interpolate(0.0, 4.0 * interface_scale, corner_progress),
        shoulder_y,
        corner_radius,
        lower_corner_y: height - corner_radius,
        lower_inner_edge: panel_inset + corner_radius,
        lower_outer_edge: panel_right_inset - corner_radius,
        right_outer_control: interpolate(
            panel_right_inset,
            panel_width - 7.0 * interface_scale,
            corner_progress,
        ),
        right_outer_edge: interpolate(panel_right_inset, panel_width, corner_progress),
        bottom: height,
    }
}

#[cfg(windows)]
fn apply_native_panel_region(
    window: &WebviewWindow,
    panel_width: f64,
    interface_scale: f64,
    window_height: f64,
    content_height: f64,
    collapsed_height: f64,
    collapsed_corner_progress: f64,
) -> Result<(), String> {
    use windows::Win32::{
        Foundation::POINT,
        Graphics::Gdi::{
            BeginPath, CloseFigure, DeleteObject, EndPath, GetDC, LineTo, MoveToEx, PathToRegion,
            PolyBezierTo, ReleaseDC, SetWindowRgn, HGDIOBJ,
        },
    };

    let hwnd = window.hwnd().map_err(|error| error.to_string())?;
    let scale_factor = window.scale_factor().map_err(|error| error.to_string())?;
    let shape = panel_shape_geometry(
        panel_width,
        interface_scale,
        window_height,
        content_height,
        collapsed_height,
        collapsed_corner_progress,
    );
    let panel_inset = PANEL_INSET * interface_scale;
    let panel_right_inset = panel_width - panel_inset;
    let point = |x: f64, y: f64| POINT {
        x: (x * scale_factor).round() as i32,
        y: (y * scale_factor).round() as i32,
    };

    unsafe {
        let hdc = GetDC(Some(hwnd));
        if hdc.0.is_null() {
            return Err("Unable to acquire the native panel device context".to_string());
        }

        let region_result = (|| {
            if !BeginPath(hdc).as_bool()
                || !MoveToEx(hdc, point(shape.outer_edge, 0.0).x, 0, None).as_bool()
                || !PolyBezierTo(
                    hdc,
                    &[
                        point(shape.outer_control, 0.0),
                        point(panel_inset, shape.shoulder_control_y),
                        point(panel_inset, shape.shoulder_y),
                    ],
                )
                .as_bool()
                || !LineTo(
                    hdc,
                    point(panel_inset, shape.lower_corner_y).x,
                    point(panel_inset, shape.lower_corner_y).y,
                )
                .as_bool()
            {
                return Err("Unable to begin the native panel region path".to_string());
            }

            let lower_curve_control_y = shape.lower_corner_y + shape.corner_radius * 2.0 / 3.0;
            let lower_curve_control_x = panel_inset + shape.corner_radius / 3.0;
            if !PolyBezierTo(
                hdc,
                &[
                    point(panel_inset, lower_curve_control_y),
                    point(lower_curve_control_x, shape.bottom),
                    point(shape.lower_inner_edge, shape.bottom),
                ],
            )
            .as_bool()
                || !LineTo(
                    hdc,
                    point(shape.lower_outer_edge, shape.bottom).x,
                    point(shape.lower_outer_edge, shape.bottom).y,
                )
                .as_bool()
            {
                return Err("Unable to draw the native panel lower-left edge".to_string());
            }

            let right_curve_control_x = panel_right_inset - shape.corner_radius / 3.0;
            if !PolyBezierTo(
                hdc,
                &[
                    point(right_curve_control_x, shape.bottom),
                    point(panel_right_inset, lower_curve_control_y),
                    point(panel_right_inset, shape.lower_corner_y),
                ],
            )
            .as_bool()
                || !LineTo(
                    hdc,
                    point(panel_right_inset, shape.shoulder_y).x,
                    point(panel_right_inset, shape.shoulder_y).y,
                )
                .as_bool()
                || !PolyBezierTo(
                    hdc,
                    &[
                        point(panel_right_inset, shape.shoulder_control_y),
                        point(shape.right_outer_control, 0.0),
                        point(shape.right_outer_edge, 0.0),
                    ],
                )
                .as_bool()
                || !CloseFigure(hdc).as_bool()
                || !EndPath(hdc).as_bool()
            {
                return Err("Unable to complete the native panel region path".to_string());
            }

            let region = PathToRegion(hdc);
            if region.0.is_null() {
                return Err("Unable to create the native panel region".to_string());
            }
            Ok(region)
        })();

        ReleaseDC(Some(hwnd), hdc);
        let region = region_result?;
        if SetWindowRgn(hwnd, Some(region), true) == 0 {
            let _ = DeleteObject(HGDIOBJ(region.0));
            return Err("Unable to apply the native panel region".to_string());
        }
    }

    Ok(())
}

#[cfg(not(windows))]
fn apply_native_panel_region(
    _window: &WebviewWindow,
    _panel_width: f64,
    _interface_scale: f64,
    _window_height: f64,
    _content_height: f64,
    _collapsed_height: f64,
    _collapsed_corner_progress: f64,
) -> Result<(), String> {
    Ok(())
}

fn horizontal_top_position(
    monitor_position: PhysicalPosition<i32>,
    monitor_size: PhysicalSize<u32>,
    window_width: u32,
    horizontal_position: f64,
) -> PhysicalPosition<i32> {
    let horizontal_position = if horizontal_position.is_finite() {
        horizontal_position.clamp(0.0, 1.0)
    } else {
        0.5
    };
    let available_width = i64::from(monitor_size.width) - i64::from(window_width);
    let x = i64::from(monitor_position.x)
        + (available_width as f64 * horizontal_position).round() as i64;

    PhysicalPosition::new(x as i32, monitor_position.y)
}

#[cfg(test)]
fn centered_top_position(
    monitor_position: PhysicalPosition<i32>,
    monitor_size: PhysicalSize<u32>,
    window_width: u32,
) -> PhysicalPosition<i32> {
    horizontal_top_position(monitor_position, monitor_size, window_width, 0.5)
}

fn monitor_signature(window: &WebviewWindow) -> Result<Option<MonitorSignature>, String> {
    let monitor = window
        .primary_monitor()
        .map_err(|error| error.to_string())?;

    Ok(monitor.map(|monitor| {
        let position = monitor.position();
        let size = monitor.size();

        MonitorSignature {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
            scale_factor: monitor.scale_factor(),
        }
    }))
}

fn center_on_primary_monitor(window: &WebviewWindow) -> Result<(), String> {
    position_on_primary_monitor(window, 0.5)
}

fn position_on_primary_monitor(
    window: &WebviewWindow,
    horizontal_position: f64,
) -> Result<(), String> {
    let monitor = window
        .primary_monitor()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "No primary monitor is available".to_string())?;
    let window_size = window.outer_size().map_err(|error| error.to_string())?;
    let position = horizontal_top_position(
        *monitor.position(),
        *monitor.size(),
        window_size.width,
        horizontal_position,
    );

    let _position_guard = PANEL_POSITION_LOCK
        .lock()
        .map_err(|error| error.to_string())?;
    window
        .set_position(position)
        .map_err(|error| error.to_string())
}

fn animate_panel_position(
    window: WebviewWindow,
    generation: u64,
    start: PhysicalPosition<i32>,
    target: PhysicalPosition<i32>,
    duration: Duration,
) -> Result<bool, String> {
    let started_at = Instant::now();

    loop {
        let progress = (started_at.elapsed().as_secs_f64() / duration.as_secs_f64()).min(1.0);
        let eased_progress = ease_out_quad(progress);
        let position = PhysicalPosition::new(
            interpolate(start.x as f64, target.x as f64, eased_progress).round() as i32,
            interpolate(start.y as f64, target.y as f64, eased_progress).round() as i32,
        );
        let _position_guard = PANEL_POSITION_LOCK
            .lock()
            .map_err(|error| error.to_string())?;

        if PANEL_POSITION_ANIMATION_GENERATION.load(Ordering::Acquire) != generation {
            return Ok(false);
        }

        window
            .set_position(position)
            .map_err(|error| error.to_string())?;
        if progress >= 1.0 {
            return Ok(true);
        }

        drop(_position_guard);
        thread::sleep(ANIMATION_FRAME_INTERVAL);
    }
}

#[tauri::command]
async fn set_panel_horizontal_position(
    window: WebviewWindow,
    state: tauri::State<'_, PanelWindowState>,
    horizontal_position: f64,
    animate: bool,
) -> Result<f64, String> {
    let horizontal_position = if horizontal_position.is_finite() {
        horizontal_position.clamp(0.0, 1.0)
    } else {
        0.5
    };
    let monitor = window
        .primary_monitor()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "No primary monitor is available".to_string())?;
    let window_size = window.outer_size().map_err(|error| error.to_string())?;
    let target = horizontal_top_position(
        *monitor.position(),
        *monitor.size(),
        window_size.width,
        horizontal_position,
    );
    let generation = PANEL_POSITION_ANIMATION_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;

    let start = window.outer_position().map_err(|error| error.to_string())?;
    let completed = if animate && start != target {
        tauri::async_runtime::spawn_blocking(move || {
            animate_panel_position(window, generation, start, target, POSITION_DURATION)
        })
        .await
        .map_err(|error| error.to_string())??
    } else {
        let _position_guard = PANEL_POSITION_LOCK
            .lock()
            .map_err(|error| error.to_string())?;
        if PANEL_POSITION_ANIMATION_GENERATION.load(Ordering::Acquire) != generation {
            false
        } else {
            window
                .set_position(target)
                .map_err(|error| error.to_string())?;
            true
        }
    };

    if completed {
        *state
            .horizontal_position
            .lock()
            .map_err(|error| error.to_string())? = horizontal_position;
    }
    Ok(horizontal_position)
}

#[tauri::command]
fn move_panel_horizontally(
    window: WebviewWindow,
    state: tauri::State<'_, PanelWindowState>,
    delta: f64,
) -> Result<f64, String> {
    PANEL_POSITION_ANIMATION_GENERATION.fetch_add(1, Ordering::AcqRel);
    let monitor = window
        .primary_monitor()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "No primary monitor is available".to_string())?;
    let window_size = window.outer_size().map_err(|error| error.to_string())?;
    let current_position = window.outer_position().map_err(|error| error.to_string())?;
    let scale_factor = window.scale_factor().map_err(|error| error.to_string())?;
    let available_width = i64::from(monitor.size().width) - i64::from(window_size.width);
    let left_endpoint = i64::from(monitor.position().x);
    let right_endpoint = left_endpoint + available_width;
    let minimum_x = left_endpoint.min(right_endpoint);
    let maximum_x = left_endpoint.max(right_endpoint);
    let physical_delta = if delta.is_finite() {
        (delta * scale_factor).round() as i64
    } else {
        0
    };
    let next_x = (i64::from(current_position.x) + physical_delta).clamp(minimum_x, maximum_x);
    let horizontal_position = if available_width == 0 {
        0.5
    } else {
        ((next_x - left_endpoint) as f64 / available_width as f64).clamp(0.0, 1.0)
    };

    let _position_guard = PANEL_POSITION_LOCK
        .lock()
        .map_err(|error| error.to_string())?;
    window
        .set_position(PhysicalPosition::new(next_x as i32, monitor.position().y))
        .map_err(|error| error.to_string())?;
    *state
        .horizontal_position
        .lock()
        .map_err(|error| error.to_string())? = horizontal_position;
    Ok(horizontal_position)
}

#[cfg(windows)]
fn configure_native_window(window: &WebviewWindow) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE,
        SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER,
        WINDOW_EX_STYLE, WINDOW_STYLE, WS_EX_APPWINDOW, WS_EX_CLIENTEDGE, WS_EX_DLGMODALFRAME,
        WS_EX_STATICEDGE, WS_EX_TOOLWINDOW, WS_EX_WINDOWEDGE, WS_OVERLAPPEDWINDOW, WS_POPUP,
    };

    let hwnd = window.hwnd().map_err(|error| error.to_string())?;

    // Make the panel a captionless tool popup while keeping it focusable for form controls.
    // Merely disabling Tauri decorations can leave a native caption behind when Windows
    // redraws the 5px-tall transparent window.
    unsafe {
        let style = (WINDOW_STYLE(GetWindowLongPtrW(hwnd, GWL_STYLE) as u32)
            & !WS_OVERLAPPEDWINDOW)
            | WS_POPUP;
        SetWindowLongPtrW(hwnd, GWL_STYLE, style.0 as isize);

        let extended_style = WINDOW_EX_STYLE(GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32);
        let native_edges =
            WS_EX_DLGMODALFRAME | WS_EX_CLIENTEDGE | WS_EX_STATICEDGE | WS_EX_WINDOWEDGE;
        let extended_style = (extended_style | WS_EX_TOOLWINDOW) & !WS_EX_APPWINDOW & !native_edges;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, extended_style.0 as isize);

        SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED
                | SWP_NOMOVE
                | SWP_NOSIZE
                | SWP_NOZORDER
                | SWP_NOOWNERZORDER
                | SWP_NOACTIVATE,
        )
        .map_err(|error| error.to_string())?;
    }

    Ok(())
}

#[cfg(not(windows))]
fn configure_native_window(_window: &WebviewWindow) -> Result<(), String> {
    Ok(())
}

/// Show the panel while preserving whichever application currently owns the
/// foreground focus. The panel remains focusable, so a user click can still
/// activate it and interact with its controls normally.
#[cfg(windows)]
fn show_native_window_without_activation(window: &WebviewWindow) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, ShowWindow, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE,
        SWP_NOZORDER, SWP_SHOWWINDOW, SW_SHOWNOACTIVATE,
    };

    let hwnd = window.hwnd().map_err(|error| error.to_string())?;

    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_SHOWWINDOW
                | SWP_NOMOVE
                | SWP_NOSIZE
                | SWP_NOZORDER
                | SWP_NOOWNERZORDER
                | SWP_NOACTIVATE,
        )
        .map_err(|error| error.to_string())?;
    }

    Ok(())
}

#[cfg(not(windows))]
fn show_native_window_without_activation(window: &WebviewWindow) -> Result<(), String> {
    window.show().map_err(|error| error.to_string())
}

fn ease_in_quad(progress: f64) -> f64 {
    progress * progress
}

fn ease_out_quad(progress: f64) -> f64 {
    1.0 - (1.0 - progress).powi(2)
}

fn animation_height(start: f64, target: f64, progress: f64, growing: bool) -> f64 {
    let eased_progress = if growing {
        ease_out_quad(progress)
    } else {
        ease_in_quad(progress)
    };

    start + (target - start) * eased_progress
}

fn panel_animation_duration(growing: bool) -> Duration {
    match growing {
        true => GROW_DURATION,
        false => SHRINK_DURATION,
    }
}

fn normalized_panel_width(panel_width: f64) -> f64 {
    if panel_width.is_finite() {
        panel_width.clamp(MIN_PANEL_WIDTH, MAX_PANEL_WIDTH)
    } else {
        PANEL_WIDTH
    }
}

fn normalized_interface_scale(interface_scale: f64) -> f64 {
    if interface_scale.is_finite() {
        interface_scale.clamp(0.75, 1.5)
    } else {
        1.0
    }
}

fn normalized_expanded_height(content_height: f64, interface_scale: f64) -> f64 {
    let collapsed_height = COLLAPSED_HEIGHT * interface_scale;
    if content_height.is_finite() {
        content_height.clamp(collapsed_height, MAX_PANEL_HEIGHT)
    } else {
        collapsed_height
    }
}

fn normalized_collapsed_height(requested: Option<f64>, interface_scale: f64) -> f64 {
    let minimum_height = COLLAPSED_HEIGHT * interface_scale;
    requested
        .filter(|value| value.is_finite())
        .map(|value| value.clamp(minimum_height, MAX_PANEL_HEIGHT))
        .unwrap_or(minimum_height)
}

fn animate_panel_height(
    window: WebviewWindow,
    generation: u64,
    panel_width: f64,
    interface_scale: f64,
    start_height: f64,
    target_height: f64,
    content_height: f64,
    collapsed_height: f64,
    collapsed_corner_progress: f64,
    growing: bool,
    duration: Duration,
) -> Result<(), String> {
    let started_at = Instant::now();

    loop {
        let progress = (started_at.elapsed().as_secs_f64() / duration.as_secs_f64()).min(1.0);
        let height = animation_height(start_height, target_height, progress, growing);
        let _resize_guard = PANEL_RESIZE_LOCK
            .lock()
            .map_err(|error| error.to_string())?;

        if PANEL_ANIMATION_GENERATION.load(Ordering::Acquire) != generation {
            return Ok(());
        }

        window
            .set_size(LogicalSize::new(panel_width, height))
            .map_err(|error| error.to_string())?;
        apply_native_panel_region(
            &window,
            panel_width,
            interface_scale,
            height,
            content_height,
            collapsed_height,
            collapsed_corner_progress,
        )?;
        drop(_resize_guard);

        if progress >= 1.0 {
            return Ok(());
        }

        thread::sleep(ANIMATION_FRAME_INTERVAL);
    }
}

#[tauri::command]
async fn set_panel_expanded(
    window: WebviewWindow,
    shape_state: tauri::State<'_, PanelShapeState>,
    expanded: bool,
    panel_width: f64,
    interface_scale: f64,
    height: f64,
    collapsed_height: Option<f64>,
    collapsed_corner_progress: Option<f64>,
    animate_height: bool,
) -> Result<(), String> {
    let generation = PANEL_ANIMATION_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    let panel_width = normalized_panel_width(panel_width);
    let interface_scale = normalized_interface_scale(interface_scale);
    let content_height = normalized_expanded_height(height, interface_scale);
    let collapsed_height = normalized_collapsed_height(collapsed_height, interface_scale);
    let collapsed_corner_progress = normalized_corner_progress(collapsed_corner_progress);
    {
        let mut request = shape_state
            .request
            .lock()
            .map_err(|error| error.to_string())?;
        *request = Some(PanelShapeRequest {
            panel_width,
            interface_scale,
            content_height,
            collapsed_height,
            collapsed_corner_progress,
        });
    }
    let target_height = if expanded {
        content_height
    } else {
        collapsed_height
    };

    let start_height = {
        let _resize_guard = PANEL_RESIZE_LOCK
            .lock()
            .map_err(|error| error.to_string())?;
        let scale_factor = window.scale_factor().map_err(|error| error.to_string())?;

        window
            .inner_size()
            .map_err(|error| error.to_string())?
            .to_logical::<f64>(scale_factor)
            .height
    };
    let growing = target_height >= start_height;

    if !animate_height {
        let _resize_guard = PANEL_RESIZE_LOCK
            .lock()
            .map_err(|error| error.to_string())?;
        if PANEL_ANIMATION_GENERATION.load(Ordering::Acquire) != generation {
            return Ok(());
        }
        window
            .set_size(LogicalSize::new(panel_width, target_height))
            .map_err(|error| error.to_string())?;
        return apply_native_panel_region(
            &window,
            panel_width,
            interface_scale,
            target_height,
            content_height,
            collapsed_height,
            collapsed_corner_progress,
        );
    }

    tauri::async_runtime::spawn_blocking(move || {
        animate_panel_height(
            window,
            generation,
            panel_width,
            interface_scale,
            start_height,
            target_height,
            content_height,
            collapsed_height,
            collapsed_corner_progress,
            growing,
            panel_animation_duration(growing),
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn show_panel_for_attention(window: WebviewWindow) -> Result<(), String> {
    show_native_window_without_activation(&window)
}

fn configure_main_window(window: &WebviewWindow) -> Result<(), String> {
    window
        .set_decorations(false)
        .map_err(|error| error.to_string())?;
    window
        .set_focusable(true)
        .map_err(|error| error.to_string())?;
    window
        .set_min_size(Some(LogicalSize::new(MIN_PANEL_WIDTH, MIN_PANEL_HEIGHT)))
        .map_err(|error| error.to_string())?;
    window
        .set_max_size(Some(LogicalSize::new(MAX_PANEL_WIDTH, MAX_PANEL_HEIGHT)))
        .map_err(|error| error.to_string())?;
    configure_native_window(window)?;
    window
        .set_size(LogicalSize::new(PANEL_WIDTH, COLLAPSED_HEIGHT))
        .map_err(|error| error.to_string())?;
    apply_native_panel_region(
        window,
        PANEL_WIDTH,
        1.0,
        COLLAPSED_HEIGHT,
        COLLAPSED_HEIGHT,
        COLLAPSED_HEIGHT,
        0.0,
    )?;
    center_on_primary_monitor(window)
}

async fn restore_main_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        return show_native_window_without_activation(&window);
    }
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|config| config.label == "main")
        .cloned()
        .ok_or_else(|| "Main window configuration is unavailable".to_string())?;
    let window = WebviewWindowBuilder::from_config(&app, &config)
        .map_err(|error| error.to_string())?
        .build()
        .map_err(|error| error.to_string())?;
    configure_main_window(&window)?;
    show_native_window_without_activation(&window)?;
    watch_primary_monitor(window);
    Ok(())
}

fn minimal_mode_menu_available(mode: approval_policy::ApprovalMode) -> bool {
    matches!(
        mode,
        approval_policy::ApprovalMode::Risk | approval_policy::ApprovalMode::Automatic
    )
}

fn build_tray_menu(
    app: &tauri::AppHandle,
    active_session_count: usize,
    approval_settings: &approval_policy::ApprovalSettings,
) -> tauri::Result<Menu<tauri::Wry>> {
    let active_sessions = MenuItem::with_id(
        app,
        "active-sessions",
        format!("活跃会话 {active_session_count}"),
        false,
        None::<&str>,
    )?;
    let settings = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let minimal_mode = CheckMenuItem::with_id(
        app,
        "minimal-mode",
        "极简模式",
        true,
        approval_settings.minimal_mode,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;

    if minimal_mode_menu_available(approval_settings.mode) {
        Menu::with_items(app, &[&active_sessions, &settings, &minimal_mode, &quit])
    } else {
        Menu::with_items(app, &[&active_sessions, &settings, &quit])
    }
}

fn refresh_tray_menu(app: &tauri::AppHandle) -> Result<(), String> {
    let approval_settings = app
        .state::<ApprovalIntegrationState>()
        .settings
        .lock()
        .map_err(|error| error.to_string())?
        .clone();
    let active_session_count = app
        .state::<TrayMenuState>()
        .active_session_count
        .load(Ordering::Relaxed);
    let menu = build_tray_menu(app, active_session_count, &approval_settings)
        .map_err(|error| error.to_string())?;
    if let Some(tray) = app.tray_by_id(TRAY_ICON_ID) {
        tray.set_menu(Some(menu))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn schedule_main_window_close_for_minimal_mode(app: tauri::AppHandle) {
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(160));
        let still_minimal = app
            .state::<ApprovalIntegrationState>()
            .settings
            .lock()
            .map(|settings| settings.minimal_mode)
            .unwrap_or(false);
        if still_minimal {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.close();
            }
        }
    });
}

async fn show_main_window(app: tauri::AppHandle) -> Result<(), String> {
    restore_main_window(app.clone()).await?;
    if let Some(window) = app.get_webview_window("main") {
        window.set_focus().map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn persist_minimal_mode(
    app: &tauri::AppHandle,
    enabled: bool,
) -> Result<approval_policy::ApprovalSettings, String> {
    let state = app.state::<ApprovalIntegrationState>();
    let current = state
        .settings
        .lock()
        .map_err(|error| error.to_string())?
        .clone();
    let enabled = enabled && minimal_mode_menu_available(current.mode);
    if current.minimal_mode == enabled {
        return Ok(current);
    }

    let next = approval_policy::ApprovalSettings {
        mode: current.mode,
        minimal_mode: enabled,
    };
    approval_policy::save_settings(&next)?;
    *state.settings.lock().map_err(|error| error.to_string())? = next.clone();
    let _ = app.emit(APPROVAL_SETTINGS_CHANGED_EVENT, &next);
    if let Err(error) = refresh_tray_menu(app) {
        eprintln!("Unable to refresh the tray menu: {error}");
    }
    Ok(next)
}

fn request_settings_from_tray(app: &tauri::AppHandle) {
    if let Err(error) = persist_minimal_mode(app, false) {
        eprintln!("Unable to exit minimal mode before opening settings: {error}");
    }
    OPEN_SETTINGS_REQUESTED.store(true, Ordering::Release);
    let _ = app.emit(OPEN_SETTINGS_REQUESTED_EVENT, ());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = show_main_window(app).await {
            eprintln!("Unable to open the settings window: {error}");
        }
    });
}

fn toggle_minimal_mode_from_tray(app: &tauri::AppHandle) {
    let enabled = app
        .state::<ApprovalIntegrationState>()
        .settings
        .lock()
        .map(|settings| !settings.minimal_mode)
        .unwrap_or(false);
    let settings = match persist_minimal_mode(app, enabled) {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("Unable to update minimal mode from the tray: {error}");
            let _ = refresh_tray_menu(app);
            return;
        }
    };
    if settings.minimal_mode {
        schedule_main_window_close_for_minimal_mode(app.clone());
    } else {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = show_main_window(app).await {
                eprintln!("Unable to restore the main window: {error}");
            }
        });
    }
}

#[tauri::command]
fn take_reopen_request() -> bool {
    REOPEN_REQUESTED.swap(false, Ordering::AcqRel)
}

#[tauri::command]
fn take_open_settings_request() -> bool {
    OPEN_SETTINGS_REQUESTED.swap(false, Ordering::AcqRel)
}

fn codex_window_score(title: &str, process_name: &str, hints: &[String]) -> u16 {
    let title = title.to_ascii_lowercase();
    let process_name = process_name.to_ascii_lowercase();
    let is_codex_process = process_name == "codex.exe";
    let is_terminal_process = process_name.contains("terminal")
        || process_name.contains("wezterm")
        || process_name.contains("alacritty");
    let mentions_codex = title.contains("codex");
    let requires_attention = title.contains("action required");
    let matches_hint = hints.iter().any(|hint| {
        let hint = hint.trim().to_ascii_lowercase();
        hint.len() >= 3 && title.contains(&hint)
    });

    if !is_codex_process
        && !mentions_codex
        && !requires_attention
        && !(is_terminal_process && matches_hint)
    {
        return 0;
    }

    (if is_codex_process { 100 } else { 0 })
        + (if requires_attention { 80 } else { 0 })
        + (if mentions_codex { 60 } else { 0 })
        + (if matches_hint { 40 } else { 0 })
        + (if is_terminal_process { 10 } else { 0 })
}

fn zcode_window_score(title: &str, process_name: &str) -> u16 {
    let title = title.to_ascii_lowercase();
    let process_name = process_name.to_ascii_lowercase();
    let is_zcode_process = process_name == "zcode.exe";
    let mentions_zcode = title.contains("zcode");

    if !is_zcode_process && !mentions_zcode {
        return 0;
    }

    (if is_zcode_process { 100 } else { 0 }) + (if mentions_zcode { 60 } else { 0 })
}

#[cfg(windows)]
fn focus_codex_window_native(
    current_window: &WebviewWindow,
    hints: Vec<String>,
) -> Result<(), String> {
    use windows::{
        core::{BOOL, PWSTR},
        Win32::{
            Foundation::{CloseHandle, HWND, LPARAM},
            System::Threading::{
                OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
            UI::WindowsAndMessaging::{
                EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
                SetForegroundWindow, ShowWindow, SW_RESTORE,
            },
        },
    };

    struct SearchContext {
        current_hwnd: HWND,
        hints: Vec<String>,
        best: Option<(HWND, u16)>,
    }

    unsafe fn process_name(hwnd: HWND) -> String {
        let mut process_id = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));
        if process_id == 0 {
            return String::new();
        }

        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) else {
            return String::new();
        };
        let mut buffer = vec![0u16; 32_768];
        let mut length = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        );
        let _ = CloseHandle(process);
        if result.is_err() {
            return String::new();
        }

        let path = String::from_utf16_lossy(&buffer[..length as usize]);
        Path::new(&path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string()
    }

    unsafe extern "system" fn visit_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let context = &mut *(lparam.0 as *mut SearchContext);
        if hwnd == context.current_hwnd || !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1);
        }

        let mut title_buffer = [0u16; 512];
        let title_length = GetWindowTextW(hwnd, &mut title_buffer);
        if title_length <= 0 {
            return BOOL(1);
        }

        let title = String::from_utf16_lossy(&title_buffer[..title_length as usize]);
        let score = codex_window_score(&title, &process_name(hwnd), &context.hints);
        if score
            > context
                .best
                .map(|(_, best_score)| best_score)
                .unwrap_or_default()
        {
            context.best = Some((hwnd, score));
        }
        BOOL(1)
    }

    let mut context = SearchContext {
        current_hwnd: current_window.hwnd().map_err(|error| error.to_string())?,
        hints,
        best: None,
    };

    unsafe {
        EnumWindows(
            Some(visit_window),
            LPARAM((&mut context as *mut SearchContext) as isize),
        )
        .map_err(|error| error.to_string())?;
    }

    let (target, _) = context
        .best
        .ok_or_else(|| "未找到可切换的 Codex 窗口".to_string())?;
    unsafe {
        if IsIconic(target).as_bool() {
            let _ = ShowWindow(target, SW_RESTORE);
        }
        if !SetForegroundWindow(target).as_bool() {
            return Err("无法将 Codex 窗口切换到前台".to_string());
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn focus_codex_window_native(
    _current_window: &WebviewWindow,
    _hints: Vec<String>,
) -> Result<(), String> {
    Err("当前平台不支持切换到 Codex 窗口".to_string())
}

#[tauri::command]
fn focus_codex_window(
    window: WebviewWindow,
    title_hint: Option<String>,
    cwd_hint: Option<String>,
) -> Result<(), String> {
    let hints = [title_hint, cwd_hint]
        .into_iter()
        .flatten()
        .filter(|hint| !hint.trim().is_empty())
        .collect();
    focus_codex_window_native(&window, hints)
}

#[cfg(windows)]
fn focus_zcode_window_native(current_window: &WebviewWindow) -> Result<(), String> {
    use windows::{
        core::{BOOL, PWSTR},
        Win32::{
            Foundation::{CloseHandle, HWND, LPARAM},
            System::Threading::{
                OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
            UI::WindowsAndMessaging::{
                EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
                SetForegroundWindow, ShowWindow, SW_RESTORE,
            },
        },
    };

    struct SearchContext {
        current_hwnd: HWND,
        best: Option<(HWND, u16)>,
    }

    unsafe fn process_name(hwnd: HWND) -> String {
        let mut process_id = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));
        if process_id == 0 {
            return String::new();
        }

        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) else {
            return String::new();
        };
        let mut buffer = vec![0u16; 32_768];
        let mut length = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        );
        let _ = CloseHandle(process);
        if result.is_err() {
            return String::new();
        }

        let path = String::from_utf16_lossy(&buffer[..length as usize]);
        Path::new(&path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string()
    }

    unsafe extern "system" fn visit_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let context = &mut *(lparam.0 as *mut SearchContext);
        if hwnd == context.current_hwnd || !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1);
        }

        let mut title_buffer = [0u16; 512];
        let title_length = GetWindowTextW(hwnd, &mut title_buffer);
        let title = if title_length > 0 {
            String::from_utf16_lossy(&title_buffer[..title_length as usize])
        } else {
            String::new()
        };
        let score = zcode_window_score(&title, &process_name(hwnd));
        if score
            > context
                .best
                .map(|(_, best_score)| best_score)
                .unwrap_or_default()
        {
            context.best = Some((hwnd, score));
        }
        BOOL(1)
    }

    let mut context = SearchContext {
        current_hwnd: current_window.hwnd().map_err(|error| error.to_string())?,
        best: None,
    };

    unsafe {
        EnumWindows(
            Some(visit_window),
            LPARAM((&mut context as *mut SearchContext) as isize),
        )
        .map_err(|error| error.to_string())?;
    }

    let (target, _) = context
        .best
        .ok_or_else(|| "未找到可切换的 ZCode 窗口".to_string())?;
    unsafe {
        if IsIconic(target).as_bool() {
            let _ = ShowWindow(target, SW_RESTORE);
        }
        if !SetForegroundWindow(target).as_bool() {
            return Err("无法将 ZCode 窗口切换到前台".to_string());
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn focus_zcode_window_native(_current_window: &WebviewWindow) -> Result<(), String> {
    Err("当前平台不支持切换到 ZCode 窗口".to_string())
}

#[tauri::command]
fn focus_zcode_window(window: WebviewWindow) -> Result<(), String> {
    focus_zcode_window_native(&window)
}

#[tauri::command]
fn list_claude_sessions(
    state: tauri::State<'_, ClaudeIntegrationState>,
) -> Result<ClaudeSessionSnapshot, String> {
    claude_snapshot(&state)
}

/// Shared snapshot builder so the desktop panel and the LAN console always read
/// the same session state instead of maintaining two code paths.
fn claude_snapshot(state: &ClaudeIntegrationState) -> Result<ClaudeSessionSnapshot, String> {
    let mut session_store = state.sessions.lock().map_err(|error| error.to_string())?;
    session_store.drain_inbox()?;

    let integration_error = state
        .hook_error
        .lock()
        .map_err(|error| error.to_string())?
        .clone();

    Ok(ClaudeSessionSnapshot {
        connected: integration_error.is_none(),
        integration_error,
        sessions: session_store.sessions(),
    })
}

#[tauri::command]
fn claude_hook_install(state: tauri::State<'_, ClaudeIntegrationState>) -> Result<(), String> {
    let _lock = hook_configuration_lock()?;
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    claude_hook::install_claude_hooks(&executable)?;
    save_hook_installation_state(HookAgentId::ClaudeCode, true)?;
    *state.hook_error.lock().map_err(|error| error.to_string())? = None;
    Ok(())
}

#[tauri::command]
async fn list_hook_integrations(
    app: tauri::AppHandle,
) -> Result<Vec<HookIntegrationStatus>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<CodexIntegrationState>();
        let opencode_state = app.state::<OpenCodeIntegrationState>();
        let mimo_state = app.state::<MimoIntegrationState>();
        let dsh_state = app.state::<DshIntegrationState>();
        let zcode_state = app.state::<ZCodeIntegrationState>();
        hook_statuses(
            &state,
            &opencode_state,
            &mimo_state,
            &dsh_state,
            &zcode_state,
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn pi_hook_status() -> Result<pi_hook::PiHookStatus, String> {
    pi_hook::status()
}

#[tauri::command]
fn pi_hook_install() -> Result<pi_hook::PiHookStatus, String> {
    let _lock = hook_configuration_lock()?;
    pi_hook::install()
}

#[tauri::command]
fn pi_hook_uninstall() -> Result<(), String> {
    let _lock = hook_configuration_lock()?;
    pi_hook::uninstall()
}

#[tauri::command]
fn list_pi_sessions(state: tauri::State<'_, PiIntegrationState>) -> Result<pi::PiSnapshot, String> {
    pi_snapshot(&state)
}

pub(crate) fn pi_snapshot(state: &PiIntegrationState) -> Result<pi::PiSnapshot, String> {
    let status = pi_hook::status()?;
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    if !status.installed() {
        store.clear();
        let error = status.error.or_else(|| Some("PI Hook 未安装".to_string()));
        store.set_integration_error(error);
        return store.snapshot();
    }
    let hook_error = state
        .hook_error
        .lock()
        .map_err(|error| error.to_string())?
        .clone();
    store.set_integration_error(hook_error);
    store.snapshot()
}

#[tauri::command]
fn pi_respond_approval(
    state: tauri::State<'_, PiIntegrationState>,
    extension_instance_id: String,
    session_id: String,
    request_id: String,
    decision: pi::PiApprovalDecision,
) -> Result<(), String> {
    apply_pi_approval(
        &state,
        &extension_instance_id,
        &session_id,
        &request_id,
        decision,
    )
}

pub(crate) fn apply_pi_approval(
    state: &PiIntegrationState,
    extension_instance_id: &str,
    session_id: &str,
    request_id: &str,
    decision: pi::PiApprovalDecision,
) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    store.drain_inbox()?;
    store.submit_approval(extension_instance_id, session_id, request_id, decision)
}

#[tauri::command]
fn pi_respond_question(
    state: tauri::State<'_, PiIntegrationState>,
    extension_instance_id: String,
    session_id: String,
    request_id: String,
    answers: Vec<pi::PiQuestionAnswer>,
) -> Result<(), String> {
    apply_pi_question(
        &state,
        &extension_instance_id,
        &session_id,
        &request_id,
        answers,
    )
}

#[tauri::command]
fn list_dsh_sessions(
    state: tauri::State<'_, DshIntegrationState>,
) -> Result<dsh::DshSnapshot, String> {
    dsh_snapshot(&state)
}

pub(crate) fn dsh_snapshot(state: &DshIntegrationState) -> Result<dsh::DshSnapshot, String> {
    let status = dsh_hook::status()?;
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    if !status.installed() {
        store.clear();
        store.set_integration_error(
            status
                .error
                .or_else(|| Some("DeepSeek Harness Hook 未安装".to_string())),
        );
    } else {
        let hook_error = state
            .hook_error
            .lock()
            .map_err(|error| error.to_string())?
            .clone();
        store.set_integration_error(hook_error);
    }
    Ok(store.snapshot())
}

fn deliver_dsh_response(
    state: &DshIntegrationState,
    request_id: &str,
    response: serde_json::Value,
) -> Result<(), String> {
    let sender = state
        .pending
        .lock()
        .map_err(|error| error.to_string())?
        .remove(request_id)
        .ok_or_else(|| "The DSH bridge request is no longer connected".to_string())?;
    let delivered = sender.send(response).is_ok();
    state
        .store
        .lock()
        .map_err(|error| error.to_string())?
        .resolve(request_id);
    if delivered {
        Ok(())
    } else {
        Err("The DSH bridge disconnected before applying the decision".to_string())
    }
}

#[tauri::command]
fn dsh_respond_approval(
    state: tauri::State<'_, DshIntegrationState>,
    bridge_instance_id: String,
    plugin_instance_id: String,
    session_id: String,
    request_id: String,
    decision: dsh::DshApprovalDecision,
) -> Result<(), String> {
    apply_dsh_approval(
        &state,
        &bridge_instance_id,
        &plugin_instance_id,
        &session_id,
        &request_id,
        decision,
    )
}

pub(crate) fn apply_dsh_approval(
    state: &DshIntegrationState,
    bridge_instance_id: &str,
    plugin_instance_id: &str,
    session_id: &str,
    request_id: &str,
    decision: dsh::DshApprovalDecision,
) -> Result<(), String> {
    let response = state
        .store
        .lock()
        .map_err(|error| error.to_string())?
        .approval_response(
            bridge_instance_id,
            plugin_instance_id,
            session_id,
            request_id,
            decision,
        )?;
    deliver_dsh_response(state, request_id, response)
}

#[tauri::command]
fn dsh_respond_question(
    state: tauri::State<'_, DshIntegrationState>,
    bridge_instance_id: String,
    plugin_instance_id: String,
    session_id: String,
    request_id: String,
    answers: Vec<dsh::DshQuestionAnswer>,
) -> Result<(), String> {
    apply_dsh_question(
        &state,
        &bridge_instance_id,
        &plugin_instance_id,
        &session_id,
        &request_id,
        answers,
    )
}

pub(crate) fn apply_dsh_question(
    state: &DshIntegrationState,
    bridge_instance_id: &str,
    plugin_instance_id: &str,
    session_id: &str,
    request_id: &str,
    answers: Vec<dsh::DshQuestionAnswer>,
) -> Result<(), String> {
    let response = state
        .store
        .lock()
        .map_err(|error| error.to_string())?
        .question_response(
            bridge_instance_id,
            plugin_instance_id,
            session_id,
            request_id,
            answers,
        )?;
    deliver_dsh_response(state, request_id, response)
}

#[tauri::command]
fn dsh_respond_plan(
    state: tauri::State<'_, DshIntegrationState>,
    bridge_instance_id: String,
    plugin_instance_id: String,
    session_id: String,
    request_id: String,
    approved: bool,
    feedback: Option<String>,
) -> Result<(), String> {
    apply_dsh_plan(
        &state,
        &bridge_instance_id,
        &plugin_instance_id,
        &session_id,
        &request_id,
        approved,
        feedback,
    )
}

pub(crate) fn apply_dsh_plan(
    state: &DshIntegrationState,
    bridge_instance_id: &str,
    plugin_instance_id: &str,
    session_id: &str,
    request_id: &str,
    approved: bool,
    feedback: Option<String>,
) -> Result<(), String> {
    let response = state
        .store
        .lock()
        .map_err(|error| error.to_string())?
        .plan_response(
            bridge_instance_id,
            plugin_instance_id,
            session_id,
            request_id,
            approved,
            feedback,
        )?;
    deliver_dsh_response(state, request_id, response)
}

#[tauri::command]
fn list_zcode_sessions(
    state: tauri::State<'_, ZCodeIntegrationState>,
) -> Result<zcode::ZCodeSnapshot, String> {
    zcode_snapshot(&state)
}

pub(crate) fn zcode_snapshot(
    state: &ZCodeIntegrationState,
) -> Result<zcode::ZCodeSnapshot, String> {
    let status = zcode_hook::status()?;
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    if !status.installed() {
        store.clear();
        store.set_integration_error(status.error.or_else(|| {
            Some(match status.state {
                zcode_hook::ZCodeHookInstallState::Incompatible => {
                    "当前 ZCode 版本与 CodeCraft Hook 不兼容".to_string()
                }
                zcode_hook::ZCodeHookInstallState::Modified => {
                    "ZCode Hook 配置已被修改，需要修复".to_string()
                }
                zcode_hook::ZCodeHookInstallState::Conflict => {
                    "检测到重复的 CodeCraft ZCode Hook".to_string()
                }
                _ => "ZCode Hook 未安装".to_string(),
            })
        }));
    } else {
        store.drain_inbox()?;
        store.set_integration_error(
            state
                .hook_error
                .lock()
                .map_err(|error| error.to_string())?
                .clone(),
        );
    }
    Ok(store.snapshot(status.detected_path, status.detected_version))
}

#[tauri::command]
fn zcode_respond_approval(
    state: tauri::State<'_, ZCodeIntegrationState>,
    session_id: String,
    request_id: String,
    decision: zcode_hook::ZCodeApprovalDecision,
    message: Option<String>,
) -> Result<(), String> {
    apply_zcode_approval(&state, &session_id, &request_id, decision, message)
}

pub(crate) fn apply_zcode_approval(
    state: &ZCodeIntegrationState,
    session_id: &str,
    request_id: &str,
    decision: zcode_hook::ZCodeApprovalDecision,
    message: Option<String>,
) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    store.drain_inbox()?;
    store.submit_permission(session_id, request_id, decision, message)
}

#[tauri::command]
fn zcode_respond_question(
    state: tauri::State<'_, ZCodeIntegrationState>,
    session_id: String,
    request_id: String,
    answers: Vec<zcode_hook::ZCodeQuestionAnswer>,
    annotations: Option<serde_json::Value>,
) -> Result<(), String> {
    apply_zcode_question(&state, &session_id, &request_id, answers, annotations)
}

pub(crate) fn apply_zcode_question(
    state: &ZCodeIntegrationState,
    session_id: &str,
    request_id: &str,
    answers: Vec<zcode_hook::ZCodeQuestionAnswer>,
    annotations: Option<serde_json::Value>,
) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    store.drain_inbox()?;
    store.submit_question(session_id, request_id, answers, annotations)
}

#[tauri::command]
fn zcode_respond_plan(
    state: tauri::State<'_, ZCodeIntegrationState>,
    session_id: String,
    request_id: String,
    approved: bool,
    feedback: Option<String>,
) -> Result<(), String> {
    apply_zcode_plan(&state, &session_id, &request_id, approved, feedback)
}

pub(crate) fn apply_zcode_plan(
    state: &ZCodeIntegrationState,
    session_id: &str,
    request_id: &str,
    approved: bool,
    feedback: Option<String>,
) -> Result<(), String> {
    let feedback_for_zcode = zcode_plan_feedback(approved, feedback.as_deref());
    {
        let mut store = state.store.lock().map_err(|error| error.to_string())?;
        store.drain_inbox()?;
        store.submit_plan(session_id, request_id, approved, feedback)?;
    }
    if let Some(feedback) = feedback_for_zcode {
        zcode_hook::queue_plan_feedback(session_id, &feedback);
    }
    Ok(())
}

fn zcode_plan_feedback(approved: bool, feedback: Option<&str>) -> Option<String> {
    (!approved)
        .then(|| {
            feedback
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .flatten()
}

pub(crate) fn apply_pi_question(
    state: &PiIntegrationState,
    extension_instance_id: &str,
    session_id: &str,
    request_id: &str,
    answers: Vec<pi::PiQuestionAnswer>,
) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    store.drain_inbox()?;
    store.submit_question(extension_instance_id, session_id, request_id, answers)
}

#[tauri::command]
fn list_opencode_sessions(
    state: tauri::State<'_, OpenCodeIntegrationState>,
) -> Result<opencode::OpenCodeSnapshot, String> {
    opencode_snapshot(&state)
}

pub(crate) fn opencode_snapshot(
    state: &OpenCodeIntegrationState,
) -> Result<opencode::OpenCodeSnapshot, String> {
    let hook_error = state
        .hook_error
        .lock()
        .map_err(|error| error.to_string())?
        .clone();
    state
        .store
        .lock()
        .map_err(|error| error.to_string())?
        .snapshot(hook_error)
}

#[tauri::command]
fn submit_opencode_question(
    plugin_instance_id: String,
    session_id: String,
    request_id: String,
    answers: Vec<Vec<String>>,
    state: tauri::State<'_, OpenCodeIntegrationState>,
) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    store.drain_inbox()?;
    store.submit_question(&plugin_instance_id, &session_id, &request_id, answers)
}

#[tauri::command]
fn reject_opencode_question(
    plugin_instance_id: String,
    session_id: String,
    request_id: String,
    state: tauri::State<'_, OpenCodeIntegrationState>,
) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    store.drain_inbox()?;
    store.reject_question(&plugin_instance_id, &session_id, &request_id)
}

#[tauri::command]
fn submit_opencode_permission(
    plugin_instance_id: String,
    session_id: String,
    request_id: String,
    action: String,
    message: Option<String>,
    state: tauri::State<'_, OpenCodeIntegrationState>,
) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    store.drain_inbox()?;
    store.submit_permission(
        &plugin_instance_id,
        &session_id,
        &request_id,
        &action,
        message,
    )
}

#[tauri::command]
fn submit_opencode_tool_gate(
    plugin_instance_id: String,
    session_id: String,
    review_id: String,
    action: String,
    state: tauri::State<'_, OpenCodeIntegrationState>,
) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    store.drain_inbox()?;
    store.submit_gate(&plugin_instance_id, &session_id, &review_id, &action)
}

#[tauri::command]
fn list_mimo_sessions(
    state: tauri::State<'_, MimoIntegrationState>,
) -> Result<mimo::MimoSnapshot, String> {
    mimo_snapshot(&state)
}

pub(crate) fn mimo_snapshot(
    state: &MimoIntegrationState,
) -> Result<mimo::MimoSnapshot, String> {
    let hook_error = state
        .hook_error
        .lock()
        .map_err(|error| error.to_string())?
        .clone();
    state
        .store
        .lock()
        .map_err(|error| error.to_string())?
        .snapshot(hook_error)
}

#[tauri::command]
fn mimo_respond_question(
    plugin_instance_id: String,
    session_id: String,
    request_id: String,
    answers: Vec<Vec<String>>,
    state: tauri::State<'_, MimoIntegrationState>,
) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    store.drain_inbox()?;
    store.submit_question(&plugin_instance_id, &session_id, &request_id, answers)
}

#[tauri::command]
fn mimo_reject_question(
    plugin_instance_id: String,
    session_id: String,
    request_id: String,
    state: tauri::State<'_, MimoIntegrationState>,
) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    store.drain_inbox()?;
    store.reject_question(&plugin_instance_id, &session_id, &request_id)
}

#[tauri::command]
fn mimo_respond_approval(
    plugin_instance_id: String,
    session_id: String,
    request_id: String,
    action: String,
    message: Option<String>,
    state: tauri::State<'_, MimoIntegrationState>,
) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    store.drain_inbox()?;
    store.submit_permission(&plugin_instance_id, &session_id, &request_id, &action, message)
}

#[tauri::command]
fn mimo_respond_gate(
    plugin_instance_id: String,
    session_id: String,
    review_id: String,
    action: String,
    state: tauri::State<'_, MimoIntegrationState>,
) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    store.drain_inbox()?;
    store.submit_gate(&plugin_instance_id, &session_id, &review_id, &action)
}

#[tauri::command]
fn mimo_respond_plan(
    plugin_instance_id: String,
    session_id: String,
    request_id: String,
    approved: bool,
    feedback: Option<String>,
    state: tauri::State<'_, MimoIntegrationState>,
) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    store.drain_inbox()?;
    store.submit_plan(&plugin_instance_id, &session_id, &request_id, approved, feedback)
}

async fn wait_for_opencode_decision(
    state: &OpenCodeIntegrationState,
    decision_id: &str,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        {
            let mut store = state.store.lock().map_err(|error| error.to_string())?;
            store.drain_inbox()?;
            if let Some(receipt) = store.take_decision_receipt(decision_id) {
                return if receipt.result == "applied" {
                    Ok(())
                } else {
                    Err(receipt
                        .error
                        .unwrap_or_else(|| "OpenCode rejected the session action".to_string()))
                };
            }
        }
        if Instant::now() >= deadline {
            return Err("Timed out waiting for OpenCode to apply the session action".to_string());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tauri::command]
async fn switch_opencode_agent(
    plugin_instance_id: String,
    session_id: String,
    agent: String,
    state: tauri::State<'_, OpenCodeIntegrationState>,
) -> Result<(), String> {
    let decision_id = {
        let mut store = state.store.lock().map_err(|error| error.to_string())?;
        store.drain_inbox()?;
        store.switch_agent(&plugin_instance_id, &session_id, &agent)?
    };
    wait_for_opencode_decision(&state, &decision_id).await
}

#[tauri::command]
async fn send_opencode_session_message(
    plugin_instance_id: String,
    session_id: String,
    message: String,
    state: tauri::State<'_, OpenCodeIntegrationState>,
) -> Result<(), String> {
    let decision_id = {
        let mut store = state.store.lock().map_err(|error| error.to_string())?;
        store.drain_inbox()?;
        store.send_message(&plugin_instance_id, &session_id, &message)?
    };
    wait_for_opencode_decision(&state, &decision_id).await
}

#[tauri::command]
async fn install_agent_hook(agent: HookAgentId, app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _lock = hook_configuration_lock()?;
        let claude_state = app.state::<ClaudeIntegrationState>();
        let codex_state = app.state::<CodexIntegrationState>();
        let opencode_state = app.state::<OpenCodeIntegrationState>();
        let mimo_state = app.state::<MimoIntegrationState>();
        let pi_state = app.state::<PiIntegrationState>();
        let dsh_state = app.state::<DshIntegrationState>();
        let zcode_state = app.state::<ZCodeIntegrationState>();
        if !agent_is_installed(agent, &codex_state)? {
            return Err(format!("{} 未安装", agent.display_name()));
        }

        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        match agent {
            HookAgentId::ClaudeCode => {
                claude_hook::install_claude_hooks(&executable)?;
                save_hook_installation_state(agent, true)?;
                *claude_state
                    .hook_error
                    .lock()
                    .map_err(|error| error.to_string())? = None;
            }
            HookAgentId::Codex => {
                let project_dir = codex_hook_project_dir(&codex_state)?;
                install_codex_hooks(&executable, project_dir.as_deref().map(Path::new))?;
                save_hook_installation_state(agent, true)?;
                *codex_state
                    .hook_error
                    .lock()
                    .map_err(|error| error.to_string())? = None;
                codex_state
                    .hook_store
                    .lock()
                    .map_err(|error| error.to_string())?
                    .set_integration_error(None);
            }
            HookAgentId::OpenCode => {
                let status = opencode_hook::install()?;
                save_hook_installation_state(agent, true)?;
                *opencode_state
                    .hook_error
                    .lock()
                    .map_err(|error| error.to_string())? = status.error;
            }
            HookAgentId::Mimo => {
                let status = mimo_hook::install()?;
                save_hook_installation_state(agent, true)?;
                *mimo_state
                    .hook_error
                    .lock()
                    .map_err(|error| error.to_string())? = status.error;
            }
            HookAgentId::Pi => {
                let status = pi_hook::install()?;
                save_hook_installation_state(agent, true)?;
                *pi_state
                    .hook_error
                    .lock()
                    .map_err(|error| error.to_string())? = status.error;
            }
            HookAgentId::DeepSeekHarness => {
                let status = dsh_hook::install()?;
                save_hook_installation_state(agent, true)?;
                *dsh_state
                    .hook_error
                    .lock()
                    .map_err(|error| error.to_string())? = status.error;
            }
            HookAgentId::ZCode => {
                let status = zcode_hook::install(&executable)?;
                save_hook_installation_state(agent, true)?;
                *zcode_state
                    .hook_error
                    .lock()
                    .map_err(|error| error.to_string())? = status.error;
            }
        }
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn uninstall_agent_hook(agent: HookAgentId, app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _lock = hook_configuration_lock()?;
        let claude_state = app.state::<ClaudeIntegrationState>();
        let codex_state = app.state::<CodexIntegrationState>();
        let opencode_state = app.state::<OpenCodeIntegrationState>();
        let mimo_state = app.state::<MimoIntegrationState>();
        let pi_state = app.state::<PiIntegrationState>();
        let dsh_state = app.state::<DshIntegrationState>();
        let zcode_state = app.state::<ZCodeIntegrationState>();
        if !agent_is_installed(agent, &codex_state)? {
            return Err(format!("{} 未安装", agent.display_name()));
        }

        match agent {
            HookAgentId::ClaudeCode => {
                claude_hook::uninstall_claude_hooks()?;
                save_hook_installation_state(agent, false)?;
                *claude_state
                    .hook_error
                    .lock()
                    .map_err(|error| error.to_string())? =
                    Some("Claude Code Hook 未安装".to_string());
            }
            HookAgentId::Codex => {
                let project_dir = codex_hook_project_dir(&codex_state)?;
                codex_hook::uninstall_codex_hooks(project_dir.as_deref().map(Path::new))?;
                save_hook_installation_state(agent, false)?;
                let message = "Codex Hook 未安装".to_string();
                *codex_state
                    .hook_error
                    .lock()
                    .map_err(|error| error.to_string())? = Some(message.clone());
                let mut store = codex_state
                    .hook_store
                    .lock()
                    .map_err(|error| error.to_string())?;
                store.clear();
                store.set_integration_error(Some(message));
            }
            HookAgentId::OpenCode => {
                opencode_hook::uninstall()?;
                save_hook_installation_state(agent, false)?;
                *opencode_state
                    .hook_error
                    .lock()
                    .map_err(|error| error.to_string())? = Some("OpenCode Hook 未安装".to_string());
            }
            HookAgentId::Mimo => {
                mimo_hook::uninstall()?;
                save_hook_installation_state(agent, false)?;
                *mimo_state
                    .hook_error
                    .lock()
                    .map_err(|error| error.to_string())? = Some("Mimo Hook 未安装".to_string());
            }
            HookAgentId::Pi => {
                pi_hook::uninstall()?;
                save_hook_installation_state(agent, false)?;
                *pi_state
                    .hook_error
                    .lock()
                    .map_err(|error| error.to_string())? = Some("PI Hook 未安装".to_string());
                pi_state
                    .store
                    .lock()
                    .map_err(|error| error.to_string())?
                    .clear();
            }
            HookAgentId::DeepSeekHarness => {
                dsh_hook::uninstall()?;
                save_hook_installation_state(agent, false)?;
                *dsh_state
                    .hook_error
                    .lock()
                    .map_err(|error| error.to_string())? =
                    Some("DeepSeek Harness Hook 未安装".to_string());
                dsh_state
                    .store
                    .lock()
                    .map_err(|error| error.to_string())?
                    .clear();
            }
            HookAgentId::ZCode => {
                zcode_hook::uninstall()?;
                save_hook_installation_state(agent, false)?;
                *zcode_state
                    .hook_error
                    .lock()
                    .map_err(|error| error.to_string())? = Some("ZCode Hook 未安装".to_string());
                zcode_state
                    .store
                    .lock()
                    .map_err(|error| error.to_string())?
                    .clear();
            }
        }
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn get_approval_settings(
    state: tauri::State<'_, ApprovalIntegrationState>,
) -> Result<approval_policy::ApprovalSettings, String> {
    Ok(state
        .settings
        .lock()
        .map_err(|error| error.to_string())?
        .clone())
}

#[tauri::command]
fn set_approval_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, ApprovalIntegrationState>,
    mode: approval_policy::ApprovalMode,
    minimal_mode: Option<bool>,
) -> Result<approval_policy::ApprovalSettings, String> {
    let current = state
        .settings
        .lock()
        .map_err(|error| error.to_string())?
        .clone();
    let minimal_mode = if mode == approval_policy::ApprovalMode::Manual {
        false
    } else {
        minimal_mode.unwrap_or(current.minimal_mode)
    };
    let settings = approval_policy::ApprovalSettings { mode, minimal_mode };
    approval_policy::save_settings(&settings)?;
    opencode_hook::sync_approval_mode(settings.mode)?;
    *state.settings.lock().map_err(|error| error.to_string())? = settings.clone();
    let _ = app.emit(APPROVAL_SETTINGS_CHANGED_EVENT, &settings);
    if let Err(error) = refresh_tray_menu(&app) {
        eprintln!("Unable to refresh the tray approval state: {error}");
    }
    if let Some(window) = app.get_webview_window("main") {
        if settings.minimal_mode {
            schedule_main_window_close_for_minimal_mode(app.clone());
        } else {
            show_native_window_without_activation(&window)?;
        }
    }
    Ok(settings)
}

#[tauri::command]
fn set_native_sound_settings(
    state: tauri::State<'_, NativeSoundIntegrationState>,
    enabled: bool,
    volume: f32,
    pack: native_sound::SoundPack,
) -> Result<native_sound::NativeSoundSettings, String> {
    let mut settings = state.settings.lock().map_err(|error| error.to_string())?;
    settings.enabled = enabled;
    settings.volume = volume.clamp(0.0, 1.0);
    settings.pack = pack;
    native_sound::save_settings(&settings)?;
    Ok(settings.clone())
}

#[tauri::command]
fn set_native_custom_sound(
    state: tauri::State<'_, NativeSoundIntegrationState>,
    event: native_sound::SoundEvent,
    file_name: String,
    bytes: Vec<u8>,
) -> Result<native_sound::NativeSoundSettings, String> {
    let mut settings = state.settings.lock().map_err(|error| error.to_string())?;
    native_sound::save_custom_sound(event, &file_name, &bytes, &mut settings)?;
    Ok(settings.clone())
}

#[tauri::command]
fn submit_claude_question_answer(
    request_id: String,
    answers: Vec<claude_hook::ClaudeQuestionAnswer>,
) -> Result<(), String> {
    claude_hook::submit_question_answer(&request_id, &answers)
}

#[tauri::command]
fn submit_claude_permission_decision(
    state: tauri::State<'_, ClaudeIntegrationState>,
    request_id: String,
    decision: claude_hook::PermissionDecision,
) -> Result<(), String> {
    apply_claude_permission_decision(&state, &request_id, decision)
}

#[tauri::command]
fn submit_claude_plan_decision(
    state: tauri::State<'_, ClaudeIntegrationState>,
    request_id: String,
    mode: claude_hook::PlanExecutionMode,
    note: Option<String>,
) -> Result<(), String> {
    apply_claude_plan_decision(&state, &request_id, mode, note)
}

/// Writes a Claude permission decision and clears it from the store so both the
/// desktop panel and the LAN console stay idempotent.
fn apply_claude_permission_decision(
    state: &ClaudeIntegrationState,
    request_id: &str,
    decision: claude_hook::PermissionDecision,
) -> Result<(), String> {
    claude_hook::submit_permission_decision(request_id, decision)?;
    let mut session_store = state.sessions.lock().map_err(|error| error.to_string())?;
    session_store.clear_permission(request_id);
    Ok(())
}

fn apply_claude_plan_decision(
    state: &ClaudeIntegrationState,
    request_id: &str,
    mode: claude_hook::PlanExecutionMode,
    note: Option<String>,
) -> Result<(), String> {
    claude_hook::submit_plan_decision(request_id, mode, note)?;
    let mut session_store = state.sessions.lock().map_err(|error| error.to_string())?;
    session_store.clear_plan(request_id);
    Ok(())
}

#[tauri::command]
fn list_codex_sessions(
    state: tauri::State<'_, CodexIntegrationState>,
) -> Result<codex::CodexSnapshot, String> {
    codex_snapshot(&state)
}

/// Shared snapshot builder for the desktop panel and the LAN console.
fn codex_snapshot(state: &CodexIntegrationState) -> Result<codex::CodexSnapshot, String> {
    let project_dir = codex_hook_project_dir(state)?;
    let hook_status = codex_hook::codex_hooks_installed(project_dir.as_deref().map(Path::new));

    // Codex state is exclusively hook-derived. Fail closed so an uninstall or
    // an invalid hooks.json cannot leave stale sessions visible in either UI.
    let hook_installed = match hook_status {
        Ok(installed) => installed,
        Err(error) => {
            let mut store = state.hook_store.lock().map_err(|lock| lock.to_string())?;
            let _ = codex_hook::drain_inbox_events();
            store.clear();
            store.set_integration_error(Some(error));
            return Ok(store.snapshot());
        }
    };

    if !hook_installed {
        let mut store = state.hook_store.lock().map_err(|error| error.to_string())?;
        let _ = codex_hook::drain_inbox_events();
        store.clear();
        store.set_integration_error(Some("Codex Hook 未安装".to_string()));
        return Ok(store.snapshot());
    }

    drain_codex_hook_events(state)?;
    let integration_error = state
        .hook_error
        .lock()
        .map_err(|error| error.to_string())?
        .clone();
    let mut store = state.hook_store.lock().map_err(|error| error.to_string())?;
    store.set_integration_error(integration_error);
    Ok(store.snapshot())
}

fn drain_codex_hook_events(state: &CodexIntegrationState) -> Result<usize, String> {
    let project_dir = codex_hook_project_dir(state)?;
    if !codex_hook::codex_hooks_installed(project_dir.as_deref().map(Path::new))? {
        let _ = codex_hook::drain_inbox_events();
        *state.hook_error.lock().map_err(|error| error.to_string())? =
            Some("Codex Hook 未安装".to_string());
        let mut store = state.hook_store.lock().map_err(|error| error.to_string())?;
        store.clear();
        store.set_integration_error(Some("Codex Hook 未安装".to_string()));
        return Ok(0);
    }

    let events = codex_hook::drain_inbox_events()?;
    let count = events.len();
    if count == 0 {
        return Ok(0);
    }
    {
        let mut store = state.hook_store.lock().map_err(|error| error.to_string())?;
        for event in &events {
            store.apply(event.clone());
        }
    }
    Ok(count)
}

#[tauri::command]
fn codex_hook_get_config(
    state: tauri::State<'_, CodexIntegrationState>,
) -> Result<codex_hook::CodexHookConfig, String> {
    Ok(state
        .hook_config
        .lock()
        .map_err(|error| error.to_string())?
        .clone())
}

#[tauri::command]
fn codex_hook_set_config(
    state: tauri::State<'_, CodexIntegrationState>,
    enabled: Option<bool>,
    mode: Option<codex_hook::CodexAutoApprovalMode>,
    project_dir: Option<String>,
    audit_log: Option<bool>,
) -> Result<codex_hook::CodexHookConfig, String> {
    let mut config = state
        .hook_config
        .lock()
        .map_err(|error| error.to_string())?;
    if let Some(enabled) = enabled {
        config.enabled = enabled;
    }
    if let Some(mode) = mode {
        config.mode = mode;
    }
    if let Some(project_dir) = project_dir {
        config.project_dir = if project_dir.trim().is_empty() {
            None
        } else {
            Some(project_dir.trim().to_string())
        };
    }
    if let Some(audit_log) = audit_log {
        config.audit_log = audit_log;
    }
    codex_hook::save_config(&config)?;
    Ok(config.clone())
}

#[tauri::command]
async fn codex_hook_install(
    app: tauri::AppHandle,
    project_dir: Option<String>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _lock = hook_configuration_lock()?;
        let state = app.state::<CodexIntegrationState>();
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        let selected_project = project_dir.or_else(|| {
            state
                .hook_config
                .lock()
                .ok()
                .and_then(|config| config.project_dir.clone())
        });
        let target = install_codex_hooks(
            &executable,
            selected_project.as_deref().map(std::path::Path::new),
        )?;
        save_hook_installation_state(HookAgentId::Codex, true)?;
        if let Ok(mut error) = state.hook_error.lock() {
            *error = None;
        }
        state
            .hook_store
            .lock()
            .map_err(|error| error.to_string())?
            .set_integration_error(None);
        Ok(target)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn codex_hook_drain(state: tauri::State<'_, CodexIntegrationState>) -> Result<usize, String> {
    drain_codex_hook_events(&state)
}

#[tauri::command]
fn codex_hook_audit_tail(lines: Option<usize>) -> Result<String, String> {
    codex_hook::read_audit_tail(lines.unwrap_or(80).clamp(1, 500))
}

#[tauri::command]
fn codex_respond_approval(
    state: tauri::State<'_, CodexIntegrationState>,
    request_id: String,
    decision: codex::CodexApprovalDecision,
) -> Result<(), String> {
    apply_codex_approval(&state, &request_id, decision)
}

/// Shared Codex approval path. Returning early for an already-resolved request
/// keeps repeated submissions (desktop and remote) harmless.
fn apply_codex_approval(
    state: &CodexIntegrationState,
    request_id: &str,
    decision: codex::CodexApprovalDecision,
) -> Result<(), String> {
    if !codex_hook::is_hook_approval_request(request_id) {
        return Err("not a Codex Hook approval request".to_string());
    }
    let mut store = state.hook_store.lock().map_err(|error| error.to_string())?;
    if !store.hook_approval_is_pending(request_id) {
        return Ok(());
    }
    codex_hook::submit_approval_decision(request_id, decision)?;
    store.resolve_hook_approval(request_id)?;
    Ok(())
}

fn watch_primary_monitor(window: WebviewWindow) {
    thread::spawn(move || {
        let mut previous = monitor_signature(&window).ok().flatten();

        loop {
            thread::sleep(Duration::from_secs(1));

            let current = match monitor_signature(&window) {
                Ok(signature) => signature,
                Err(_) => break,
            };

            if current != previous {
                PANEL_POSITION_ANIMATION_GENERATION.fetch_add(1, Ordering::AcqRel);
                let horizontal_position = window
                    .state::<PanelWindowState>()
                    .horizontal_position
                    .lock()
                    .map(|position| *position)
                    .unwrap_or(0.5);
                if position_on_primary_monitor(&window, horizontal_position).is_err() {
                    break;
                }
                previous = current;
            }
        }
    });
}

#[tauri::command]
fn lan_get_config(
    state: tauri::State<'_, lan_server::LanServerState>,
) -> Result<lan_config::LanServerConfig, String> {
    Ok(state
        .config
        .lock()
        .map_err(|error| error.to_string())?
        .clone())
}

/// Applies a settings change, persists it, and brings the listener in line with
/// the new configuration (start, stop, or rebind on a new port).
#[tauri::command]
fn lan_set_config(
    app: tauri::AppHandle,
    enabled: Option<bool>,
    port: Option<u16>,
    bind: Option<lan_config::LanBindMode>,
    allow_approvals: Option<bool>,
    audit_remote: Option<bool>,
) -> Result<lan_server::LanStatus, String> {
    let state = app.state::<lan_server::LanServerState>();
    let (previous, next) = {
        let mut config = state.config.lock().map_err(|error| error.to_string())?;
        let previous = config.clone();
        if let Some(port) = port {
            config.port = lan_config::validate_port(port)?;
        }
        if let Some(bind) = bind {
            config.bind = bind;
        }
        if let Some(allow_approvals) = allow_approvals {
            config.allow_approvals = allow_approvals;
        }
        if let Some(audit_remote) = audit_remote {
            config.audit_remote = audit_remote;
        }
        if let Some(enabled) = enabled {
            config.enabled = enabled;
        }
        lan_config::save_config(&config)?;
        (previous, config.clone())
    };

    if !next.enabled {
        return lan_server::stop(&app);
    }
    if !previous.enabled {
        return start_lan_server(&app);
    }
    if previous.port != next.port || previous.bind != next.bind {
        lan_server::stop(&app)?;
        return start_lan_server(&app);
    }
    lan_server::status(&state)
}

/// Starts the listener and records the failure in the config when the port is
/// unavailable, so the panel can show why the switch flipped back off.
fn start_lan_server(app: &tauri::AppHandle) -> Result<lan_server::LanStatus, String> {
    match lan_server::start(app) {
        Ok(status) => Ok(status),
        Err(error) => {
            let state = app.state::<lan_server::LanServerState>();
            {
                let mut config = state.config.lock().map_err(|error| error.to_string())?;
                config.enabled = false;
                lan_config::save_config(&config)?;
            }
            Err(error)
        }
    }
}

#[tauri::command]
fn lan_rotate_token(app: tauri::AppHandle) -> Result<String, String> {
    let state = app.state::<lan_server::LanServerState>();
    let token = lan_config::generate_token();
    {
        let mut config = state.config.lock().map_err(|error| error.to_string())?;
        config.token = token.clone();
        lan_config::save_config(&config)?;
    }
    // Existing browsers must log in again with the new token.
    state
        .auth
        .lock()
        .map_err(|error| error.to_string())?
        .invalidate_all();
    Ok(token)
}

#[tauri::command]
fn lan_status(
    state: tauri::State<'_, lan_server::LanServerState>,
) -> Result<lan_server::LanStatus, String> {
    lan_server::status(&state)
}

/// Opens the repository's releases page in the default browser so the user can
/// download the latest installer.
#[tauri::command]
fn open_release_page() -> Result<(), String> {
    open_in_default_browser("https://github.com/Turing158/CodeCraft/releases/latest")
}

#[cfg(windows)]
fn open_in_default_browser(url: &str) -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};

    let wide: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(
                "open"
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect::<Vec<u16>>()
                    .as_ptr(),
            ),
            PCWSTR(wide.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW returns a value > 32 on success.
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err("无法打开默认浏览器".to_string())
    }
}

#[cfg(not(windows))]
fn open_in_default_browser(url: &str) -> Result<(), String> {
    if std::process::Command::new("xdg-open")
        .arg(url)
        .spawn()
        .is_ok()
    {
        return Ok(());
    }
    if std::process::Command::new("open").arg(url).spawn().is_ok() {
        return Ok(());
    }
    Err("无法打开默认浏览器".to_string())
}

/// Renders the console URL as an inline SVG QR code so a phone can scan it
/// without the panel loading anything from the network.
#[tauri::command]
fn lan_address_qr_code(url: String) -> Result<String, String> {
    use qrcode::{render::svg, QrCode};

    let code = QrCode::new(url.as_bytes()).map_err(|error| error.to_string())?;
    Ok(code
        .render::<svg::Color<'_>>()
        .min_dimensions(180, 180)
        .quiet_zone(true)
        .dark_color(svg::Color("#0f172a"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

fn watch_tray_and_minimal_mode_sounds(app: tauri::AppHandle) {
    thread::spawn(move || {
        let mut observer = native_sound::NativeSoundObserver::default();
        let mut was_minimal = false;
        loop {
            let claude_state = app.state::<ClaudeIntegrationState>();
            let claude_snapshot = claude_snapshot(&claude_state);
            let codex_state = app.state::<CodexIntegrationState>();
            let codex_snapshot = codex_snapshot(&codex_state);
            let opencode_state = app.state::<OpenCodeIntegrationState>();
            let opencode_snapshot = opencode_snapshot(&opencode_state);
            let mimo_state = app.state::<MimoIntegrationState>();
            let mimo_snapshot = mimo_snapshot(&mimo_state);
            let pi_state = app.state::<PiIntegrationState>();
            let pi_snapshot = pi_snapshot(&pi_state);
            let dsh_state = app.state::<DshIntegrationState>();
            let dsh_snapshot = dsh_snapshot(&dsh_state);
            let zcode_state = app.state::<ZCodeIntegrationState>();
            let zcode_snapshot = zcode_snapshot(&zcode_state);

            if let (Ok(claude), Ok(codex), Ok(opencode), Ok(mimo), Ok(pi), Ok(dsh), Ok(zcode)) = (
                &claude_snapshot,
                &codex_snapshot,
                &opencode_snapshot,
                &mimo_snapshot,
                &pi_snapshot,
                &dsh_snapshot,
                &zcode_snapshot,
            ) {
                let active_session_count = claude
                    .sessions
                    .iter()
                    .filter(|session| session.is_active())
                    .count()
                    + codex
                        .sessions
                        .iter()
                        .filter(|session| session.is_active())
                        .count()
                    + opencode
                        .sessions
                        .iter()
                        .filter(|session| session.is_active())
                        .count()
                    + mimo
                        .sessions
                        .iter()
                        .filter(|session| session.is_active())
                        .count()
                    + pi.sessions
                        .iter()
                        .filter(|session| {
                            !matches!(
                                session.status,
                                pi::PiSessionStatus::Idle | pi::PiSessionStatus::Stopped
                            )
                        })
                        .count()
                    + dsh
                        .sessions
                        .iter()
                        .filter(|session| session.is_active())
                        .count()
                    + zcode
                        .sessions
                        .iter()
                        .filter(|session| session.is_active())
                        .count();
                let previous = app
                    .state::<TrayMenuState>()
                    .active_session_count
                    .swap(active_session_count, Ordering::Relaxed);
                if previous != active_session_count {
                    if let Err(error) = refresh_tray_menu(&app) {
                        eprintln!("Unable to refresh the tray session count: {error}");
                    }
                }
            }

            let minimal = app
                .state::<ApprovalIntegrationState>()
                .settings
                .lock()
                .map(|settings| settings.minimal_mode)
                .unwrap_or(false);
            if !minimal {
                if was_minimal {
                    observer.reset();
                }
                was_minimal = false;
            } else {
                was_minimal = true;
                let sound_enabled = app
                    .state::<NativeSoundIntegrationState>()
                    .settings
                    .lock()
                    .map(|settings| settings.enabled)
                    .unwrap_or(false);
                if !sound_enabled {
                    observer.reset();
                } else {
                    if let Ok(snapshot) = &claude_snapshot {
                        if let Ok(value) = serde_json::to_value(snapshot) {
                            observer.observe("claude", &value);
                        }
                    }
                    if let Ok(snapshot) = &codex_snapshot {
                        if let Ok(value) = serde_json::to_value(snapshot) {
                            observer.observe("codex", &value);
                        }
                    }
                    if let Ok(snapshot) = &opencode_snapshot {
                        if let Ok(value) = serde_json::to_value(snapshot) {
                            observer.observe("opencode", &value);
                        }
                    }
                    if let Ok(snapshot) = &mimo_snapshot {
                        if let Ok(value) = serde_json::to_value(snapshot) {
                            observer.observe("mimo", &value);
                        }
                    }
                    if let Ok(snapshot) = &pi_snapshot {
                        if let Ok(value) = serde_json::to_value(snapshot) {
                            observer.observe("pi", &value);
                        }
                    }
                    if let Ok(snapshot) = &dsh_snapshot {
                        if let Ok(value) = serde_json::to_value(snapshot) {
                            observer.observe("dsh", &value);
                        }
                    }
                    if let Ok(snapshot) = &zcode_snapshot {
                        if let Ok(value) = serde_json::to_value(snapshot) {
                            observer.observe("zcode", &value);
                        }
                    }
                }
            }
            thread::sleep(Duration::from_secs(1));
        }
    });
}

fn should_prevent_windowless_exit(exit_code: Option<i32>, minimal_mode: bool) -> bool {
    exit_code.is_none() && minimal_mode
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if approval_policy::load_settings().minimal_mode {
                return;
            }
            REOPEN_REQUESTED.store(true, Ordering::Release);
            let _ = app.emit(REOPEN_REQUESTED_EVENT, ());
        }))
        .manage(ClaudeIntegrationState::default())
        .manage(CodexIntegrationState::default())
        .manage(OpenCodeIntegrationState::default())
        .manage(MimoIntegrationState::default())
        .manage(PiIntegrationState::default())
        .manage(DshIntegrationState::default())
        .manage(ZCodeIntegrationState::default())
        .manage(ApprovalIntegrationState::default())
        .manage(NativeSoundIntegrationState::default())
        .manage(TrayMenuState::default())
        .manage(PanelWindowState::default())
        .manage(PanelShapeState::default())
        .manage(lan_server::LanServerState::default())
        .setup(|app| {
            let approval_settings = approval_policy::load_settings();
            *app.state::<ApprovalIntegrationState>()
                .settings
                .lock()
                .map_err(|error| error.to_string())? = approval_settings.clone();
            *app.state::<NativeSoundIntegrationState>()
                .settings
                .lock()
                .map_err(|error| error.to_string())? = native_sound::load_settings();
            let _ = claude_hook::touch_app_heartbeat();
            thread::spawn(|| loop {
                thread::sleep(Duration::from_secs(5));
                let _ = claude_hook::touch_app_heartbeat();
            });

            let mut hook_install_config = hook_config::load();
            let executable = std::env::current_exe().ok();
            let mut hook_config_changed = false;

            let dsh_state = app.state::<DshIntegrationState>();
            let bridge_error = match dsh_hook::start_bridge(app.handle().clone()) {
                Ok(bridge) => {
                    *dsh_state.bridge.lock().map_err(|error| error.to_string())? = Some(bridge);
                    None
                }
                Err(error) => Some(error),
            };

            let dsh_configured = hook_install_config.is_enabled(HookAgentId::DeepSeekHarness);
            let dsh_status = dsh_hook::status();
            let dsh_error = match dsh_status {
                Ok(status) if status.installed() => {
                    if !dsh_configured {
                        hook_install_config.set_enabled(HookAgentId::DeepSeekHarness, true);
                        hook_config_changed = true;
                    }
                    status.error
                }
                Ok(status)
                    if dsh_configured
                        && matches!(
                            status.state,
                            dsh_hook::DshHookInstallState::NotInstalled
                                | dsh_hook::DshHookInstallState::Incompatible
                        ) =>
                {
                    match dsh_hook::install() {
                        Ok(installed) => installed.error,
                        Err(error) => Some(error),
                    }
                }
                Ok(status) => status
                    .error
                    .or_else(|| Some("DeepSeek Harness Hook 未安装".to_string())),
                Err(error) => Some(error),
            };
            *dsh_state
                .hook_error
                .lock()
                .map_err(|error| error.to_string())? = bridge_error.or(dsh_error);

            let zcode_state = app.state::<ZCodeIntegrationState>();
            let zcode_configured = hook_install_config.is_enabled(HookAgentId::ZCode);
            let zcode_status = zcode_hook::status();
            let zcode_error = match zcode_status {
                Ok(status) if status.installed() => status.error,
                Ok(status)
                    if zcode_configured
                        && matches!(
                            status.state,
                            zcode_hook::ZCodeHookInstallState::NotInstalled
                                | zcode_hook::ZCodeHookInstallState::Modified
                                | zcode_hook::ZCodeHookInstallState::Incompatible
                        ) =>
                {
                    match executable.as_deref() {
                        Some(executable) => zcode_hook::install(executable)
                            .ok()
                            .and_then(|installed| installed.error),
                        None => Some("Unable to locate the CodeCraft executable".to_string()),
                    }
                }
                Ok(status) => status
                    .error
                    .or_else(|| Some("ZCode Hook 未安装".to_string())),
                Err(error) => Some(error),
            };
            *zcode_state
                .hook_error
                .lock()
                .map_err(|error| error.to_string())? = zcode_error;

            let pi_state = app.state::<PiIntegrationState>();
            let pi_configured = hook_install_config.is_enabled(HookAgentId::Pi);
            let pi_status = pi_hook::status();
            let pi_error = match pi_status {
                Ok(status) if status.installed() => {
                    if !pi_configured {
                        hook_install_config.set_enabled(HookAgentId::Pi, true);
                        hook_config_changed = true;
                    }
                    status.error
                }
                Ok(status)
                    if pi_configured
                        && matches!(
                            status.state,
                            pi_hook::PiHookInstallState::NotInstalled
                                | pi_hook::PiHookInstallState::Incompatible
                        ) =>
                {
                    match pi_hook::install() {
                        Ok(installed) => installed.error,
                        Err(error) => Some(error),
                    }
                }
                Ok(status) => status.error.or_else(|| Some("PI Hook 未安装".to_string())),
                Err(error) => Some(error),
            };
            *pi_state
                .hook_error
                .lock()
                .map_err(|error| error.to_string())? = pi_error;

            let integration_state = app.state::<ClaudeIntegrationState>();
            let claude_configured = hook_install_config.is_enabled(HookAgentId::ClaudeCode);
            let claude_hook_status = claude_hook::claude_hooks_installed();
            if claude_hook_status == Ok(true) && !claude_configured {
                hook_install_config.set_enabled(HookAgentId::ClaudeCode, true);
                hook_config_changed = true;
            }
            *integration_state
                .hook_error
                .lock()
                .map_err(|error| error.to_string())? = match claude_hook_status {
                Ok(true) => match executable.as_deref() {
                    Some(executable) => claude_hook::install_claude_hooks(executable).err(),
                    None => Some("Unable to locate the CodeCraft executable".to_string()),
                },
                Ok(false) if claude_configured => match executable.as_deref() {
                    Some(executable) => claude_hook::install_claude_hooks(executable).err(),
                    None => Some("Unable to locate the CodeCraft executable".to_string()),
                },
                Ok(false) => Some("Claude Code Hook 未安装".to_string()),
                Err(error) => Some(error),
            };

            let codex_state = app.state::<CodexIntegrationState>();
            let codex_hook_config = codex_hook::load_config();
            *codex_state
                .hook_config
                .lock()
                .map_err(|error| error.to_string())? = codex_hook_config.clone();
            let codex_project_dir = codex_hook_config.project_dir.as_deref().map(Path::new);
            let codex_configured = hook_install_config.is_enabled(HookAgentId::Codex);
            let codex_hook_status = codex_hook::codex_hooks_installed(codex_project_dir);
            if codex_hook_status == Ok(true) && !codex_configured {
                hook_install_config.set_enabled(HookAgentId::Codex, true);
                hook_config_changed = true;
            }
            let codex_hook_error = match codex_hook_status {
                Ok(true) => match executable.as_deref() {
                    Some(executable) => install_codex_hooks(executable, codex_project_dir).err(),
                    None => Some("Unable to locate the CodeCraft executable".to_string()),
                },
                Ok(false) if codex_configured => match executable.as_deref() {
                    Some(executable) => install_codex_hooks(executable, codex_project_dir).err(),
                    None => Some("Unable to locate the CodeCraft executable".to_string()),
                },
                Ok(false) => Some("Codex Hook 未安装".to_string()),
                Err(error) => Some(error),
            };
            *codex_state
                .hook_error
                .lock()
                .map_err(|error| error.to_string())? = codex_hook_error;

            let opencode_configured = hook_install_config.is_enabled(HookAgentId::OpenCode);
            let opencode_status = if opencode_configured {
                opencode_hook::status_and_sync()
            } else {
                opencode_hook::status()
            };
            let opencode_error = match opencode_status {
                Ok(status) if status.installed() => {
                    if !opencode_configured {
                        hook_install_config.set_enabled(HookAgentId::OpenCode, true);
                        hook_config_changed = true;
                    }
                    status.error
                }
                Ok(status)
                    if opencode_configured
                        && status.state
                            == opencode_hook::OpenCodeHookInstallState::NotInstalled =>
                {
                    match opencode_hook::install() {
                        Ok(installed) => installed.error,
                        Err(error) => Some(error),
                    }
                }
                Ok(status) => status
                    .error
                    .or_else(|| Some("OpenCode Hook 未安装".to_string())),
                Err(error) => Some(error),
            };
            let opencode_sync_error =
                opencode_hook::sync_approval_mode(approval_settings.mode).err();
            *app.state::<OpenCodeIntegrationState>()
                .hook_error
                .lock()
                .map_err(|error| error.to_string())? = opencode_error.or(opencode_sync_error);

            let mimo_state = app.state::<MimoIntegrationState>();
            let mimo_configured = hook_install_config.is_enabled(HookAgentId::Mimo);
            let mimo_status = if mimo_configured {
                mimo_hook::status_and_sync()
            } else {
                mimo_hook::status()
            };
            let mimo_error = match mimo_status {
                Ok(status) if status.installed() => {
                    if !mimo_configured {
                        hook_install_config.set_enabled(HookAgentId::Mimo, true);
                        hook_config_changed = true;
                    }
                    status.error
                }
                Ok(status)
                    if mimo_configured
                        && status.state == mimo_hook::MimoHookInstallState::NotInstalled =>
                {
                    match mimo_hook::install() {
                        Ok(installed) => installed.error,
                        Err(error) => Some(error),
                    }
                }
                Ok(status) => status
                    .error
                    .or_else(|| Some("Mimo Hook 未安装".to_string())),
                Err(error) => Some(error),
            };
            let mimo_sync_error = mimo_hook::sync_approval_mode(approval_settings.mode).err();
            *mimo_state
                .hook_error
                .lock()
                .map_err(|error| error.to_string())? = mimo_error.or(mimo_sync_error);

            if hook_config_changed {
                if let Err(error) = hook_config::save(&hook_install_config) {
                    eprintln!("Unable to persist hook installation config: {error}");
                }
            }

            let lan_server_config = lan_config::load_config();
            let lan_enabled = lan_server_config.enabled;
            *app.state::<lan_server::LanServerState>()
                .config
                .lock()
                .map_err(|error| error.to_string())? = lan_server_config;
            if lan_enabled {
                // A busy port must not stop the app from starting, so the
                // failure is reported through lan_status instead.
                if let Err(error) = start_lan_server(&app.handle().clone()) {
                    eprintln!("Unable to start the LAN web console: {error}");
                }
            }

            let menu = build_tray_menu(app.handle(), 0, &approval_settings)?;

            TrayIconBuilder::with_id(TRAY_ICON_ID)
                .icon(
                    app.default_window_icon()
                        .expect("the application icon must be configured")
                        .clone(),
                )
                .menu(&menu)
                .show_menu_on_left_click(false)
                .build(app)?;

            let window = app
                .get_webview_window("main")
                .expect("the main webview window must be configured");
            configure_main_window(&window)?;
            if !approval_settings.minimal_mode {
                show_native_window_without_activation(&window)?;
                watch_primary_monitor(window);
            } else {
                let app = app.handle().clone();
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(80));
                    let still_minimal = app
                        .state::<ApprovalIntegrationState>()
                        .settings
                        .lock()
                        .map(|settings| settings.minimal_mode)
                        .unwrap_or(false);
                    if still_minimal {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.close();
                        }
                    }
                });
            }
            watch_tray_and_minimal_mode_sounds(app.handle().clone());

            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }

            let tauri::WindowEvent::Resized(size) = event else {
                return;
            };
            let app_handle = window.app_handle();
            let state = app_handle.state::<PanelShapeState>();
            let request = state.request.lock().ok().and_then(|request| *request);
            let Some(request) = request else {
                return;
            };
            let Some(webview_window) = app_handle.get_webview_window("main") else {
                return;
            };
            let scale_factor = webview_window.scale_factor().unwrap_or(1.0);
            let height = size.to_logical::<f64>(scale_factor).height;
            // `set_size` can synchronously dispatch this event while the
            // animation thread still owns the resize lock. Waiting here would
            // deadlock the UI thread and leave the panel frozen on hover. The
            // active resize path applies the same region itself, so skip this
            // duplicate event when the lock is busy.
            let Ok(_resize_guard) = PANEL_RESIZE_LOCK.try_lock() else {
                return;
            };
            let _ = apply_native_panel_region(
                &webview_window,
                request.panel_width,
                request.interface_scale,
                height,
                request.content_height,
                request.collapsed_height,
                request.collapsed_corner_progress,
            );
        })
        .on_menu_event(|app, event| {
            match event.id().as_ref() {
                "settings" => request_settings_from_tray(app),
                "minimal-mode" => toggle_minimal_mode_from_tray(app),
                "quit" => {
                    // Release the LAN port before the process goes away.
                    let _ = lan_server::stop(app);
                    app.exit(0);
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            set_panel_expanded,
            set_panel_horizontal_position,
            move_panel_horizontally,
            show_panel_for_attention,
            take_reopen_request,
            take_open_settings_request,
            focus_codex_window,
            focus_zcode_window,
            get_approval_settings,
            set_approval_settings,
            set_native_sound_settings,
            set_native_custom_sound,
            list_claude_sessions,
            claude_hook_install,
            list_hook_integrations,
            pi_hook_status,
            pi_hook_install,
            pi_hook_uninstall,
            list_pi_sessions,
            pi_respond_approval,
            pi_respond_question,
            list_dsh_sessions,
            dsh_respond_approval,
            dsh_respond_question,
            dsh_respond_plan,
            list_zcode_sessions,
            zcode_respond_approval,
            zcode_respond_question,
            zcode_respond_plan,
            install_agent_hook,
            uninstall_agent_hook,
            list_opencode_sessions,
            submit_opencode_question,
            reject_opencode_question,
            submit_opencode_permission,
            submit_opencode_tool_gate,
            switch_opencode_agent,
            send_opencode_session_message,
            list_mimo_sessions,
            mimo_respond_question,
            mimo_reject_question,
            mimo_respond_approval,
            mimo_respond_gate,
            mimo_respond_plan,
            submit_claude_question_answer,
            submit_claude_permission_decision,
            submit_claude_plan_decision,
            list_codex_sessions,
            codex_respond_approval,
            codex_hook_get_config,
            codex_hook_set_config,
            codex_hook_install,
            codex_hook_drain,
            codex_hook_audit_tail,
            lan_get_config,
            lan_set_config,
            lan_rotate_token,
            lan_status,
            lan_address_qr_code,
            open_release_page
        ])
        .build(tauri::generate_context!())
        .expect("error while building CodeCraft")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
                let minimal_mode = app
                    .state::<ApprovalIntegrationState>()
                    .settings
                    .lock()
                    .map(|settings| settings.minimal_mode)
                    .unwrap_or(false);
                if should_prevent_windowless_exit(code, minimal_mode) {
                    api.prevent_exit();
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_feedback_is_queued_only_for_a_rejection_with_text() {
        assert_eq!(
            zcode_plan_feedback(false, Some("  revise the database step  ")),
            Some("revise the database step".to_string())
        );
        assert_eq!(zcode_plan_feedback(true, Some("ignored")), None);
        assert_eq!(zcode_plan_feedback(false, Some("  ")), None);
        assert_eq!(zcode_plan_feedback(false, None), None);
    }

    #[test]
    fn minimal_mode_keeps_the_backend_alive_without_a_webview() {
        assert!(should_prevent_windowless_exit(None, true));
        assert!(!should_prevent_windowless_exit(None, false));
        assert!(!should_prevent_windowless_exit(Some(0), true));
    }

    #[test]
    fn minimal_mode_menu_requires_an_automatic_approval_mode() {
        assert!(!minimal_mode_menu_available(
            approval_policy::ApprovalMode::Manual
        ));
        assert!(minimal_mode_menu_available(
            approval_policy::ApprovalMode::Risk
        ));
        assert!(minimal_mode_menu_available(
            approval_policy::ApprovalMode::Automatic
        ));
    }

    #[test]
    fn finds_codex_hosts_by_process_or_attention_title() {
        let hints = vec!["CodeCraft".to_string()];
        assert!(codex_window_score("CodeCraft", "codex.exe", &hints) > 0);
        assert!(
            codex_window_score("Action Required | CodeCraft", "WindowsTerminal.exe", &hints) > 0
        );
        assert_eq!(
            codex_window_score("CodeCraft", "codecraft-tauri.exe", &hints),
            0
        );
    }

    #[test]
    fn finds_zcode_hosts_by_process_or_title() {
        assert!(zcode_window_score("ZCode", "ZCode.exe") > 0);
        assert!(zcode_window_score("", "ZCode.exe") > 0);
        assert!(zcode_window_score("ZCode", "ZCode Helper.exe") > 0);
        assert_eq!(zcode_window_score("CodeCraft", "codecraft-tauri.exe"), 0);
    }

    #[test]
    fn centers_on_a_standard_primary_monitor() {
        let position = centered_top_position(
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(1920, 1080),
            500,
        );

        assert_eq!(position, PhysicalPosition::new(710, 0));
    }

    #[test]
    fn preserves_a_monitor_origin_above_and_left_of_zero() {
        let position = centered_top_position(
            PhysicalPosition::new(-2560, -200),
            PhysicalSize::new(2560, 1440),
            750,
        );

        assert_eq!(position, PhysicalPosition::new(-1655, -200));
    }

    #[test]
    fn centers_a_window_wider_than_the_monitor() {
        let position = centered_top_position(
            PhysicalPosition::new(300, 40),
            PhysicalSize::new(400, 900),
            500,
        );

        assert_eq!(position, PhysicalPosition::new(250, 40));
    }

    #[test]
    fn positions_a_window_at_each_horizontal_preset() {
        let monitor_position = PhysicalPosition::new(100, 40);
        let monitor_size = PhysicalSize::new(1200, 900);

        assert_eq!(
            horizontal_top_position(monitor_position, monitor_size, 500, 0.0),
            PhysicalPosition::new(100, 40),
        );
        assert_eq!(
            horizontal_top_position(monitor_position, monitor_size, 500, 0.5),
            PhysicalPosition::new(450, 40),
        );
        assert_eq!(
            horizontal_top_position(monitor_position, monitor_size, 500, 1.0),
            PhysicalPosition::new(800, 40),
        );
    }

    #[test]
    fn native_animation_preserves_its_endpoints() {
        assert_eq!(animation_height(5.0, 50.0, 0.0, true), 5.0);
        assert_eq!(animation_height(5.0, 50.0, 1.0, true), 50.0);
        assert_eq!(animation_height(50.0, 5.0, 0.0, false), 50.0);
        assert_eq!(animation_height(50.0, 5.0, 1.0, false), 5.0);
    }

    #[test]
    fn native_animation_uses_directional_easing() {
        let expanding_midpoint = animation_height(5.0, 50.0, 0.5, true);
        let collapsing_midpoint = animation_height(50.0, 5.0, 0.5, false);

        assert!(expanding_midpoint > 27.5);
        assert!(collapsing_midpoint > 27.5);
    }

    #[test]
    fn expanded_height_comes_from_measured_content() {
        assert_eq!(normalized_expanded_height(212.0, 1.0), 212.0);
        assert_eq!(normalized_expanded_height(2.0, 1.0), COLLAPSED_HEIGHT);
        assert_eq!(
            normalized_expanded_height(f64::INFINITY, 1.0),
            COLLAPSED_HEIGHT,
        );
        assert_eq!(normalized_expanded_height(2.0, 0.75), 3.75);
    }

    #[test]
    fn collapsed_height_falls_back_to_the_slim_strip() {
        assert_eq!(normalized_collapsed_height(None, 1.0), COLLAPSED_HEIGHT);
        assert_eq!(normalized_collapsed_height(Some(72.0), 1.0), 72.0);
        assert_eq!(
            normalized_collapsed_height(Some(1.0), 1.0),
            COLLAPSED_HEIGHT,
        );
        assert_eq!(normalized_collapsed_height(None, 1.5), 7.5);
    }

    #[test]
    fn reduced_interface_scale_preserves_visible_bottom_rounding() {
        let shape = panel_shape_geometry(375.0, 0.75, 108.0, 108.0, 3.75, 0.0);

        assert_eq!(shape.corner_radius, MIN_VISIBLE_BOTTOM_CORNER_RADIUS);
        assert_eq!(shape.lower_corner_y, 96.0);
    }

    #[test]
    fn short_panel_rounding_stays_below_the_upper_shoulder() {
        let shape = panel_shape_geometry(500.0, 1.0, 10.0, 10.0, 5.0, 0.0);

        assert_eq!(shape.corner_radius, 1.0);
        assert_eq!(shape.lower_corner_y, shape.shoulder_y);
    }

    #[test]
    fn two_session_sized_panel_keeps_scaled_bottom_rounding() {
        let shape = panel_shape_geometry(375.0, 0.75, 115.5, 115.5, 3.75, 0.0);

        assert_eq!(shape.corner_radius, MIN_VISIBLE_BOTTOM_CORNER_RADIUS);
        assert_eq!(shape.lower_corner_y, 103.5);
    }

    #[test]
    fn one_session_sized_panel_keeps_scaled_bottom_rounding_after_resize() {
        let shape = panel_shape_geometry(375.0, 0.75, 119.25, 119.25, 3.75, 0.0);

        assert_eq!(shape.corner_radius, MIN_VISIBLE_BOTTOM_CORNER_RADIUS);
        assert_eq!(shape.lower_corner_y, 107.25);
    }

    #[test]
    fn narrow_panel_corners_do_not_cross() {
        let shape = panel_shape_geometry(36.0, 1.0, 144.0, 144.0, 5.0, 0.0);

        assert_eq!(shape.corner_radius, 6.0);
        assert_eq!(shape.lower_inner_edge, shape.lower_outer_edge);
    }

    #[test]
    fn short_expand_animation_never_places_the_lower_curve_above_the_shoulder() {
        for height in [5.0, 5.5, 6.0, 7.0, 8.0, 9.0, 10.0, 12.0] {
            let shape = panel_shape_geometry(500.0, 1.0, height, 12.0, 5.0, 0.0);

            assert!(shape.lower_corner_y >= shape.shoulder_y);
        }
    }

    #[test]
    fn slim_collapsed_strip_uses_flat_bottom_with_rounded_top() {
        let shape = panel_shape_geometry(500.0, 1.0, 5.0, 144.0, 5.0, 0.0);

        assert_eq!(shape.corner_radius, 0.0);
        assert_eq!(shape.bottom, 5.0);
        assert_eq!(shape.lower_inner_edge, 12.0);
        assert_eq!(shape.lower_outer_edge, 488.0);
        assert_eq!(shape.outer_edge, 10.25);
        assert_eq!(shape.outer_control, 11.125);
        assert_eq!(shape.shoulder_y, 1.75);
        assert_eq!(shape.right_outer_control, 488.875);
        assert_eq!(shape.right_outer_edge, 489.75);
    }
}
