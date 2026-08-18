export type InterfaceSizingMode = "scale" | "custom";

export interface InterfaceSettings {
  mode: InterfaceSizingMode;
  scale: number;
  customFontScale: number;
  customWidth: number;
  customMinHeight: number;
  customMaxHeight: number;
  customPadding: number;
}

export const INTERFACE_SETTINGS_STORAGE_KEY = "codecraft.interface-settings";
export const BASE_PANEL_WIDTH = 500;
export const BASE_SETTINGS_MIN_HEIGHT = 300;
export const BASE_CONTENT_MAX_HEIGHT = 640;
export const BASE_SETTINGS_MAX_HEIGHT = 780;
export const MIN_INTERFACE_SCALE = 0.75;
export const MAX_INTERFACE_SCALE = 1.5;
export const MIN_FONT_SCALE = 0.75;
export const MAX_FONT_SCALE = 1.5;
export const MIN_CUSTOM_WIDTH = 360;
export const MAX_CUSTOM_WIDTH = 820;
export const MIN_CUSTOM_HEIGHT = 240;
export const MAX_CUSTOM_HEIGHT = 1100;
export const MIN_CUSTOM_PADDING = 10;
export const MAX_CUSTOM_PADDING = 32;

export const DEFAULT_INTERFACE_SETTINGS: InterfaceSettings = {
  mode: "scale",
  scale: 1,
  customFontScale: 1,
  customWidth: BASE_PANEL_WIDTH,
  customMinHeight: BASE_SETTINGS_MIN_HEIGHT,
  customMaxHeight: BASE_SETTINGS_MAX_HEIGHT,
  customPadding: 18,
};

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null;

const numberOr = (value: unknown, fallback: number) =>
  typeof value === "number" && Number.isFinite(value) ? value : fallback;

const clamp = (value: number, minimum: number, maximum: number) =>
  Math.min(maximum, Math.max(minimum, value));

export const clampInterfaceScale = (value: number) =>
  clamp(
    numberOr(value, DEFAULT_INTERFACE_SETTINGS.scale),
    MIN_INTERFACE_SCALE,
    MAX_INTERFACE_SCALE,
  );

export const clampFontScale = (value: number) =>
  clamp(
    numberOr(value, DEFAULT_INTERFACE_SETTINGS.customFontScale),
    MIN_FONT_SCALE,
    MAX_FONT_SCALE,
  );

export const normalizeInterfaceSettings = (
  value: unknown,
): InterfaceSettings => {
  if (!isRecord(value)) return { ...DEFAULT_INTERFACE_SETTINGS };

  const customMinHeight = clamp(
    numberOr(
      value.customMinHeight,
      DEFAULT_INTERFACE_SETTINGS.customMinHeight,
    ),
    MIN_CUSTOM_HEIGHT,
    MAX_CUSTOM_HEIGHT,
  );
  const requestedMaxHeight = clamp(
    numberOr(
      value.customMaxHeight,
      DEFAULT_INTERFACE_SETTINGS.customMaxHeight,
    ),
    MIN_CUSTOM_HEIGHT,
    MAX_CUSTOM_HEIGHT,
  );

  return {
    mode: value.mode === "custom" ? "custom" : "scale",
    scale: clampInterfaceScale(numberOr(value.scale, DEFAULT_INTERFACE_SETTINGS.scale)),
    customFontScale: clampFontScale(
      numberOr(
        value.customFontScale,
        DEFAULT_INTERFACE_SETTINGS.customFontScale,
      ),
    ),
    customWidth: clamp(
      numberOr(value.customWidth, DEFAULT_INTERFACE_SETTINGS.customWidth),
      MIN_CUSTOM_WIDTH,
      MAX_CUSTOM_WIDTH,
    ),
    customMinHeight,
    customMaxHeight: Math.max(customMinHeight, requestedMaxHeight),
    customPadding: clamp(
      numberOr(value.customPadding, DEFAULT_INTERFACE_SETTINGS.customPadding),
      MIN_CUSTOM_PADDING,
      MAX_CUSTOM_PADDING,
    ),
  };
};

export const parseInterfaceSettings = (rawValue: string | null) => {
  if (!rawValue) return { ...DEFAULT_INTERFACE_SETTINGS };

  try {
    return normalizeInterfaceSettings(JSON.parse(rawValue));
  } catch {
    return { ...DEFAULT_INTERFACE_SETTINGS };
  }
};

export const interfaceScaleFor = (settings: InterfaceSettings) =>
  settings.mode === "scale" ? settings.scale : 1;

export const panelWidthFor = (settings: InterfaceSettings) =>
  settings.mode === "scale" ? BASE_PANEL_WIDTH : settings.customWidth;

