export interface WindowPositionSettings {
  topDragEnabled: boolean;
  horizontalPosition: number;
}

export const WINDOW_POSITION_SETTINGS_STORAGE_KEY =
  "codecraft.window-position-settings";

export const DEFAULT_WINDOW_POSITION_SETTINGS: WindowPositionSettings = {
  topDragEnabled: false,
  horizontalPosition: 0.5,
};

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null;

export const clampHorizontalPosition = (value: number) => {
  if (!Number.isFinite(value)) {
    return DEFAULT_WINDOW_POSITION_SETTINGS.horizontalPosition;
  }
  return Math.min(1, Math.max(0, value));
};

export const normalizeWindowPositionSettings = (
  value: unknown,
): WindowPositionSettings => {
  if (!isRecord(value)) return { ...DEFAULT_WINDOW_POSITION_SETTINGS };

  return {
    topDragEnabled:
      typeof value.topDragEnabled === "boolean"
        ? value.topDragEnabled
        : DEFAULT_WINDOW_POSITION_SETTINGS.topDragEnabled,
    horizontalPosition: clampHorizontalPosition(
      typeof value.horizontalPosition === "number"
        ? value.horizontalPosition
        : DEFAULT_WINDOW_POSITION_SETTINGS.horizontalPosition,
    ),
  };
};

export const parseWindowPositionSettings = (rawValue: string | null) => {
  if (!rawValue) return { ...DEFAULT_WINDOW_POSITION_SETTINGS };

  try {
    return normalizeWindowPositionSettings(JSON.parse(rawValue));
  } catch {
    return { ...DEFAULT_WINDOW_POSITION_SETTINGS };
  }
};

