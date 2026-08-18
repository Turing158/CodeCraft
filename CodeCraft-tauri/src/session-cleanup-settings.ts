export type SessionCleanupPreset = "5m" | "10m" | "30m" | "1h" | "custom";

export interface SessionCleanupSettings {
  preset: SessionCleanupPreset;
  customMinutes: number;
}

export const SESSION_CLEANUP_SETTINGS_STORAGE_KEY =
  "codecraft.session-cleanup-settings";
export const MIN_CUSTOM_SESSION_CLEANUP_MINUTES = 1;
export const MAX_CUSTOM_SESSION_CLEANUP_MINUTES = 7 * 24 * 60;

export const DEFAULT_SESSION_CLEANUP_SETTINGS: SessionCleanupSettings = {
  preset: "30m",
  customMinutes: 30,
};

const PRESET_MINUTES: Record<Exclude<SessionCleanupPreset, "custom">, number> =
  {
    "5m": 5,
    "10m": 10,
    "30m": 30,
    "1h": 60,
  };

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null;

const isPreset = (value: unknown): value is SessionCleanupPreset =>
  value === "5m" ||
  value === "10m" ||
  value === "30m" ||
  value === "1h" ||
  value === "custom";

export const clampCustomSessionCleanupMinutes = (value: number) => {
  if (!Number.isFinite(value)) {
    return DEFAULT_SESSION_CLEANUP_SETTINGS.customMinutes;
  }
  return Math.min(
    MAX_CUSTOM_SESSION_CLEANUP_MINUTES,
    Math.max(MIN_CUSTOM_SESSION_CLEANUP_MINUTES, Math.round(value)),
  );
};

export const normalizeSessionCleanupSettings = (
  value: unknown,
): SessionCleanupSettings => {
  if (!isRecord(value)) return { ...DEFAULT_SESSION_CLEANUP_SETTINGS };

  return {
    preset: isPreset(value.preset)
      ? value.preset
      : DEFAULT_SESSION_CLEANUP_SETTINGS.preset,
    customMinutes: clampCustomSessionCleanupMinutes(
      typeof value.customMinutes === "number"
        ? value.customMinutes
        : DEFAULT_SESSION_CLEANUP_SETTINGS.customMinutes,
    ),
  };
};

export const parseSessionCleanupSettings = (rawValue: string | null) => {
  if (!rawValue) return { ...DEFAULT_SESSION_CLEANUP_SETTINGS };

  try {
    return normalizeSessionCleanupSettings(JSON.parse(rawValue));
  } catch {
    return { ...DEFAULT_SESSION_CLEANUP_SETTINGS };
  }
};

export const sessionCleanupMinutes = (settings: SessionCleanupSettings) =>
  settings.preset === "custom"
    ? clampCustomSessionCleanupMinutes(settings.customMinutes)
    : PRESET_MINUTES[settings.preset];

export const sessionCleanupAfterMs = (settings: SessionCleanupSettings) =>
  sessionCleanupMinutes(settings) * 60_000;

export const shouldAutoCleanupSession = (
  status: string,
  updatedAt: number,
  settings: SessionCleanupSettings,
  now = Date.now(),
) =>
  (status === "idle" || status === "stopped") &&
  Number.isFinite(updatedAt) &&
  now - updatedAt >= sessionCleanupAfterMs(settings);

export const filterAutoCleanedSessions = <
  T extends { status: string; updatedAt: number },
>(
  sessions: T[],
  settings: SessionCleanupSettings,
  now = Date.now(),
) =>
  sessions.filter(
    (session) =>
      !shouldAutoCleanupSession(
        session.status,
        session.updatedAt,
        settings,
        now,
      ),
  );
