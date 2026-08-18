/** Pure helpers for the desktop "LAN" settings section.
 *
 * The panel talks to Rust through Tauri commands, so everything that can be
 * decided without the DOM or a live server lives here and is unit tested.
 */

export type LanBindMode = "lan" | "loopback";

export interface LanServerConfigView {
  enabled: boolean;
  port: number;
  bind: LanBindMode;
  token: string;
  allowApprovals: boolean;
  auditRemote: boolean;
}

export interface LanAddressView {
  address: string;
  url: string;
}

export interface LanStatusView {
  running: boolean;
  port: number;
  bind: LanBindMode;
  listenAddress: string | null;
  addresses: LanAddressView[];
  clientCount: number;
  streamCount: number;
  lastClientAt: number | null;
  lastError: string | null;
  allowApprovals: boolean;
  enabled: boolean;
}

export const LAN_MIN_PORT = 1024;
export const LAN_MAX_PORT = 65535;
export const LAN_DEFAULT_PORT = 8787;

/** Marks that the plaintext-HTTP warning was already confirmed once. */
export const LAN_ACKNOWLEDGED_STORAGE_KEY = "codecraft.lan.acknowledged";

export const defaultLanConfig = (): LanServerConfigView => ({
  enabled: false,
  port: LAN_DEFAULT_PORT,
  bind: "lan",
  token: "",
  allowApprovals: false,
  auditRemote: true,
});

export const lanStatusFor = (config: LanServerConfigView): LanStatusView => ({
  running: config.enabled,
  port: config.port,
  bind: config.bind,
  listenAddress: config.enabled ? "0.0.0.0:" + config.port : null,
  addresses: config.enabled
    ? [
        {
          address: "192.168.1.20",
          url: "http://192.168.1.20:" + config.port,
        },
      ]
    : [],
  clientCount: 0,
  streamCount: 0,
  lastClientAt: null,
  lastError: null,
  allowApprovals: config.allowApprovals,
  enabled: config.enabled,
});

export type LanStateKind = "running" | "stopped" | "error";

export interface LanStateDescriptor {
  state: LanStateKind;
  label: string;
}

/**
 * A recorded error only means "stopped" once the switch is off again, so a
 * failed start still explains itself instead of silently reverting.
 */
export const lanStateDescriptor = (status: LanStatusView): LanStateDescriptor => {
  if (status.lastError && !status.running) {
    return { state: "error", label: "启动失败" };
  }
  if (status.running) {
    return {
      state: "running",
      label: status.allowApprovals ? "运行中 · 可审批" : "运行中 · 只读",
    };
  }
  return { state: "stopped", label: "未运行" };
};

export type LanPortParse =
  | { ok: true; port: number }
  | { ok: false; error: string };

export const parseLanPort = (value: string): LanPortParse => {
  const trimmed = value.trim();
  if (!/^[0-9]{1,5}$/.test(trimmed)) {
    return { ok: false, error: "端口需要是 1024–65535 之间的整数" };
  }
  const port = Number(trimmed);
  if (port < LAN_MIN_PORT || port > LAN_MAX_PORT) {
    return { ok: false, error: "端口需要在 1024–65535 之间" };
  }
  return { ok: true, port };
};

/** Shows only the tail so the panel can be read aloud or screenshotted safely. */
export const maskLanToken = (token: string): string => {
  if (!token) return "尚未生成";
  if (token.length <= 4) return "•".repeat(token.length);
  return "•".repeat(Math.min(16, token.length - 4)) + token.slice(-4);
};

export const lastClientLabel = (
  lastClientAt: number | null,
  now: number,
): string => {
  if (!lastClientAt) return "暂无访问记录";
  const elapsed = Math.max(0, now - lastClientAt);
  if (elapsed < 60_000) return "最近访问：刚刚";
  if (elapsed < 3_600_000) {
    return "最近访问：" + Math.floor(elapsed / 60_000) + " 分钟前";
  }
  if (elapsed < 86_400_000) {
    return "最近访问：" + Math.floor(elapsed / 3_600_000) + " 小时前";
  }
  return "最近访问：" + Math.floor(elapsed / 86_400_000) + " 天前";
};

/**
 * Turning the console on exposes session content over plaintext HTTP, so the
 * first attempt asks for a confirmation click instead of enabling right away.
 */
export const needsEnableConfirmation = (
  acknowledged: boolean,
  armed: boolean,
): boolean => !acknowledged && !armed;

export const ENABLE_CONFIRM_HINT =
  "开启后同一网络中拿到地址和令牌的人都能查看会话。再次点击开关以确认开启。";
export const ROTATE_CONFIRM_HINT =
  "轮换会立即让已登录的浏览器失效。再次点击“轮换”以确认。";
export const APPROVALS_RISK_HINT =
  "已允许网页提交决定：拿到令牌的人可以代你允许工具调用。";

export const lanReadOnlyHint = (status: LanStatusView): string =>
  status.allowApprovals
    ? "网页可以提交 Claude 的权限、问题与计划决定，以及 Codex 的审批。"
    : "网页当前只读，只能查看会话内容。";

/**
 * The port and the token define the listening socket and the credential that
 * clients already hold, so both are frozen while the server serves.
 */
export const lanAdvancedLocked = (status: LanStatusView): boolean =>
  status.running;

export const LAN_ADVANCED_LOCK_HINT =
  "服务运行中，端口与令牌暂不可修改。关闭服务后即可调整。";

/** One card measured just before the panel re-renders. */
export interface LanCardHeightSnapshot {
  height: number;
  hidden: boolean;
}

/** How a card should move between two measurements. */
export type LanCardHeightPlan =
  | { kind: "none" }
  | { kind: "collapse"; from: number }
  | { kind: "reveal"; from: number; to: number }
  | { kind: "resize"; from: number; to: number };

/** Sub-pixel reflow noise should not start an animation. */
export const LAN_CARD_HEIGHT_EPSILON = 0.5;

/**
 * Decides how a card should move between two measurements.
 *
 * Hiding a card sets `display: none`, so a collapse has to play while the card
 * is still part of the layout; the caller hides it once the animation ends.
 * Measuring a card that is already animating yields its in-flight height, so an
 * interrupted collapse simply gets re-planned from where it currently is.
 */
export const planLanCardHeight = (
  previous: LanCardHeightSnapshot,
  next: LanCardHeightSnapshot,
): LanCardHeightPlan => {
  if (next.hidden) {
    if (previous.hidden || previous.height < LAN_CARD_HEIGHT_EPSILON) {
      return { kind: "none" };
    }
    return { kind: "collapse", from: previous.height };
  }

  const from = previous.hidden ? 0 : previous.height;
  if (Math.abs(next.height - from) < LAN_CARD_HEIGHT_EPSILON) {
    return { kind: "none" };
  }
  return previous.hidden
    ? { kind: "reveal", from: previous.height, to: next.height }
    : { kind: "resize", from, to: next.height };
};
