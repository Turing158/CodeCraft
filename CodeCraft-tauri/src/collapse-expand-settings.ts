export interface CollapseExpandSettings {
  autoCollapseDelaySeconds: number;
  approvalAutoExpand: boolean;
}

export const COLLAPSE_EXPAND_SETTINGS_STORAGE_KEY =
  "codecraft.collapse-expand-settings";
export const MIN_AUTO_COLLAPSE_DELAY_SECONDS = 0.1;
export const MAX_AUTO_COLLAPSE_DELAY_SECONDS = 10;
export const DEFAULT_COLLAPSE_EXPAND_SETTINGS: CollapseExpandSettings = {
  autoCollapseDelaySeconds: 1,
  approvalAutoExpand: true,
};

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null;

export const clampAutoCollapseDelay = (value: number) => {
  if (!Number.isFinite(value)) {
    return DEFAULT_COLLAPSE_EXPAND_SETTINGS.autoCollapseDelaySeconds;
  }
  return Math.min(
    MAX_AUTO_COLLAPSE_DELAY_SECONDS,
    Math.max(MIN_AUTO_COLLAPSE_DELAY_SECONDS, value),
  );
};

export const normalizeCollapseExpandSettings = (
  value: unknown,
): CollapseExpandSettings => {
  if (!isRecord(value)) return { ...DEFAULT_COLLAPSE_EXPAND_SETTINGS };

  return {
    autoCollapseDelaySeconds: clampAutoCollapseDelay(
      typeof value.autoCollapseDelaySeconds === "number"
        ? value.autoCollapseDelaySeconds
        : DEFAULT_COLLAPSE_EXPAND_SETTINGS.autoCollapseDelaySeconds,
    ),
    approvalAutoExpand:
      typeof value.approvalAutoExpand === "boolean"
        ? value.approvalAutoExpand
        : DEFAULT_COLLAPSE_EXPAND_SETTINGS.approvalAutoExpand,
  };
};

export const parseCollapseExpandSettings = (rawValue: string | null) => {
  if (!rawValue) return { ...DEFAULT_COLLAPSE_EXPAND_SETTINGS };

  try {
    return normalizeCollapseExpandSettings(JSON.parse(rawValue));
  } catch {
    return { ...DEFAULT_COLLAPSE_EXPAND_SETTINGS };
  }
};

export const autoCollapseDelayMs = (settings: CollapseExpandSettings) =>
  Math.round(clampAutoCollapseDelay(settings.autoCollapseDelaySeconds) * 1_000);
