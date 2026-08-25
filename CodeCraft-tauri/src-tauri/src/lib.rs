use std::{
    env,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, WebviewWindow,
};

mod approval_policy;
mod claude_hook;
mod codex;
mod codex_hook;
mod lan_auth;
mod lan_config;
mod lan_net;
mod lan_server;
mod opencode;
mod opencode_hook;

pub use claude_hook::capture_claude_hook;
pub use codex_hook::capture_codex_hook;

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
const REOPEN_REQUESTED_EVENT: &str = "reopen-requested";

static PANEL_ANIMATION_GENERATION: AtomicU64 = AtomicU64::new(0);
static PANEL_POSITION_ANIMATION_GENERATION: AtomicU64 = AtomicU64::new(0);
static REOPEN_REQUESTED: AtomicBool = AtomicBool::new(false);
static PANEL_RESIZE_LOCK: Mutex<()> = Mutex::new(());
static PANEL_POSITION_LOCK: Mutex<()> = Mutex::new(());

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
}

impl HookAgentId {
    fn command(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
            Self::OpenCode => "OpenCode",
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
    install_state: Option<opencode_hook::OpenCodeHookInstallState>,
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
    let _ = state;
    Ok(command_is_installed(agent.command()))
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

pub(crate) fn hook_statuses(
    state: &CodexIntegrationState,
    opencode_state: &OpenCodeIntegrationState,
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
            install_state: Some(opencode.state),
            install_path: Some(opencode.install_path),
            bundled_version: Some(opencode.bundled_version.to_string()),
            installed_version: opencode.installed_version,
            running_versions: opencode.running_versions,
            error: opencode.error,
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
    window.show().map_err(|error| error.to_string())?;
    window.unminimize().map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())
}

#[tauri::command]
fn take_reopen_request() -> bool {
    REOPEN_REQUESTED.swap(false, Ordering::AcqRel)
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
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    claude_hook::install_claude_hooks(&executable)?;
    *state.hook_error.lock().map_err(|error| error.to_string())? = None;
    Ok(())
}

#[tauri::command]
fn list_hook_integrations(
    state: tauri::State<'_, CodexIntegrationState>,
    opencode_state: tauri::State<'_, OpenCodeIntegrationState>,
) -> Result<Vec<HookIntegrationStatus>, String> {
    hook_statuses(&state, &opencode_state)
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
        let claude_state = app.state::<ClaudeIntegrationState>();
        let codex_state = app.state::<CodexIntegrationState>();
        let opencode_state = app.state::<OpenCodeIntegrationState>();
        if !agent_is_installed(agent, &codex_state)? {
            return Err(format!("{} 未安装", agent.display_name()));
        }

        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        match agent {
            HookAgentId::ClaudeCode => {
                claude_hook::install_claude_hooks(&executable)?;
                *claude_state
                    .hook_error
                    .lock()
                    .map_err(|error| error.to_string())? = None;
            }
            HookAgentId::Codex => {
                let project_dir = codex_hook_project_dir(&codex_state)?;
                install_codex_hooks(&executable, project_dir.as_deref().map(Path::new))?;
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
                *opencode_state
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
        let claude_state = app.state::<ClaudeIntegrationState>();
        let codex_state = app.state::<CodexIntegrationState>();
        let opencode_state = app.state::<OpenCodeIntegrationState>();
        if !agent_is_installed(agent, &codex_state)? {
            return Err(format!("{} 未安装", agent.display_name()));
        }

        match agent {
            HookAgentId::ClaudeCode => {
                claude_hook::uninstall_claude_hooks()?;
                *claude_state
                    .hook_error
                    .lock()
                    .map_err(|error| error.to_string())? =
                    Some("Claude Code Hook 未安装".to_string());
            }
            HookAgentId::Codex => {
                let project_dir = codex_hook_project_dir(&codex_state)?;
                codex_hook::uninstall_codex_hooks(project_dir.as_deref().map(Path::new))?;
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
                *opencode_state
                    .hook_error
                    .lock()
                    .map_err(|error| error.to_string())? = Some("OpenCode Hook 未安装".to_string());
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
    state: tauri::State<'_, ApprovalIntegrationState>,
    mode: approval_policy::ApprovalMode,
) -> Result<approval_policy::ApprovalSettings, String> {
    let settings = approval_policy::ApprovalSettings { mode };
    approval_policy::save_settings(&settings)?;
    opencode_hook::sync_approval_mode(settings.mode)?;
    *state.settings.lock().map_err(|error| error.to_string())? = settings.clone();
    Ok(settings)
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
    use windows::Win32::UI::{
        Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL,
    };

    let wide: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR("open".encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>().as_ptr()),
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
    if std::process::Command::new("xdg-open").arg(url).spawn().is_ok() {
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            REOPEN_REQUESTED.store(true, Ordering::Release);
            let _ = app.emit(REOPEN_REQUESTED_EVENT, ());
        }))
        .manage(ClaudeIntegrationState::default())
        .manage(CodexIntegrationState::default())
        .manage(OpenCodeIntegrationState::default())
        .manage(ApprovalIntegrationState::default())
        .manage(PanelWindowState::default())
        .manage(PanelShapeState::default())
        .manage(lan_server::LanServerState::default())
        .setup(|app| {
            let approval_settings = approval_policy::load_settings();
            *app.state::<ApprovalIntegrationState>()
                .settings
                .lock()
                .map_err(|error| error.to_string())? = approval_settings.clone();
            let _ = claude_hook::touch_app_heartbeat();
            thread::spawn(|| loop {
                thread::sleep(Duration::from_secs(5));
                let _ = claude_hook::touch_app_heartbeat();
            });

            let integration_state = app.state::<ClaudeIntegrationState>();
            *integration_state
                .hook_error
                .lock()
                .map_err(|error| error.to_string())? = match claude_hook::claude_hooks_installed() {
                Ok(true) => std::env::current_exe()
                    .map_err(|error| error.to_string())
                    .and_then(|executable| claude_hook::install_claude_hooks(&executable))
                    .err(),
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
            let codex_hook_error = match codex_hook::codex_hooks_installed(codex_project_dir) {
                Ok(true) => std::env::current_exe()
                    .map_err(|error| error.to_string())
                    .and_then(|executable| install_codex_hooks(&executable, codex_project_dir))
                    .err(),
                Ok(false) => Some("Codex Hook 未安装".to_string()),
                Err(error) => Some(error),
            };
            *codex_state
                .hook_error
                .lock()
                .map_err(|error| error.to_string())? = codex_hook_error;

            let opencode_error = match opencode_hook::status_and_sync() {
                Ok(status) if status.installed() => status.error,
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

            let quit = MenuItem::with_id(app, "quit", "\u{9000}\u{51fa}", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&quit])?;

            TrayIconBuilder::new()
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
            window.set_decorations(false)?;
            window.set_focusable(true)?;
            window.set_min_size(Some(LogicalSize::new(MIN_PANEL_WIDTH, MIN_PANEL_HEIGHT)))?;
            window.set_max_size(Some(LogicalSize::new(MAX_PANEL_WIDTH, MAX_PANEL_HEIGHT)))?;
            configure_native_window(&window)?;
            window.set_size(LogicalSize::new(PANEL_WIDTH, COLLAPSED_HEIGHT))?;
            apply_native_panel_region(
                &window,
                PANEL_WIDTH,
                1.0,
                COLLAPSED_HEIGHT,
                COLLAPSED_HEIGHT,
                COLLAPSED_HEIGHT,
                0.0,
            )?;
            center_on_primary_monitor(&window)?;
            window.show()?;
            watch_primary_monitor(window);

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
            if event.id().as_ref() == "quit" {
                // Release the LAN port before the process goes away.
                let _ = lan_server::stop(app);
                app.exit(0);
            }
        })
        .invoke_handler(tauri::generate_handler![
            set_panel_expanded,
            set_panel_horizontal_position,
            move_panel_horizontally,
            show_panel_for_attention,
            take_reopen_request,
            focus_codex_window,
            get_approval_settings,
            set_approval_settings,
            list_claude_sessions,
            claude_hook_install,
            list_hook_integrations,
            install_agent_hook,
            uninstall_agent_hook,
            list_opencode_sessions,
            submit_opencode_question,
            reject_opencode_question,
            submit_opencode_permission,
            submit_opencode_tool_gate,
            switch_opencode_agent,
            send_opencode_session_message,
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
        .run(tauri::generate_context!())
        .expect("error while running CodeCraft");
}

#[cfg(test)]
mod tests {
    use super::*;

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
