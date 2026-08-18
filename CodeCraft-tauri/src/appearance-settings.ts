export type ThemeMode = "system" | "dark" | "light";
export type ResolvedTheme = Exclude<ThemeMode, "system">;
export type MotionMode = "system" | "on" | "off";
export const LIGHT_WORKING_SQUARE_SVGS = ["grass-block"] as const;
export const DARK_WORKING_SQUARE_SVGS = ["sea-lantern"] as const;
export type LightWorkingSquareSvg =
  (typeof LIGHT_WORKING_SQUARE_SVGS)[number];
export type DarkWorkingSquareSvg =
  (typeof DARK_WORKING_SQUARE_SVGS)[number];

export interface AppearanceSettings {
  theme: ThemeMode;
  workingSquareLightSvg: LightWorkingSquareSvg;
  workingSquareDarkSvg: DarkWorkingSquareSvg;
  workingSquareLightImageFile: string | null;
  workingSquareDarkImageFile: string | null;
  motionMode: MotionMode;
  animationSpeed: number;
  transparency: number;
  cardTransparency: number;
  textTransparency: number;
}

export const APPEARANCE_SETTINGS_STORAGE_KEY = "codecraft.appearance-settings";
export const MIN_ANIMATION_SPEED = 0.01;
export const MAX_ANIMATION_SPEED = 2;
export const MIN_TRANSPARENCY = 0.1;
export const MAX_TRANSPARENCY = 1;
export const DEFAULT_APPEARANCE_SETTINGS: AppearanceSettings = {
  theme: "dark",
  workingSquareLightSvg: "grass-block",
  workingSquareDarkSvg: "sea-lantern",
  workingSquareLightImageFile: null,
  workingSquareDarkImageFile: null,
  motionMode: "system",
  animationSpeed: 1,
  transparency: 0.88,
  cardTransparency: 0.86,
  textTransparency: 1,
};

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null;

const isLightWorkingSquareSvg = (
  value: unknown,
): value is LightWorkingSquareSvg =>
  LIGHT_WORKING_SQUARE_SVGS.some((svg) => svg === value);

const isDarkWorkingSquareSvg = (
  value: unknown,
): value is DarkWorkingSquareSvg =>
  DARK_WORKING_SQUARE_SVGS.some((svg) => svg === value);

const normalizeImageFileName = (value: unknown) =>
  typeof value === "string" && value.trim() ? value.trim() : null;

export const clampAnimationSpeed = (value: number) => {
  if (!Number.isFinite(value)) return DEFAULT_APPEARANCE_SETTINGS.animationSpeed;
  return Math.min(MAX_ANIMATION_SPEED, Math.max(MIN_ANIMATION_SPEED, value));
};

export const clampTransparency = (
  value: number,
  fallback = DEFAULT_APPEARANCE_SETTINGS.transparency,
) => {
  if (!Number.isFinite(value)) return fallback;
  return Math.min(MAX_TRANSPARENCY, Math.max(MIN_TRANSPARENCY, value));
};

export const transparencyIsActive = (value: number) =>
  clampTransparency(value) < MAX_TRANSPARENCY;

export const normalizeAppearanceSettings = (
  value: unknown,
): AppearanceSettings => {
  if (!isRecord(value)) return { ...DEFAULT_APPEARANCE_SETTINGS };

  const theme: ThemeMode =
    value.theme === "system" ||
    value.theme === "dark" ||
    value.theme === "light"
      ? value.theme
      : DEFAULT_APPEARANCE_SETTINGS.theme;
  const motionMode: MotionMode =
    value.motionMode === "on" || value.motionMode === "off"
      ? value.motionMode
      : "system";
  const workingSquareLightSvg = isLightWorkingSquareSvg(
    value.workingSquareLightSvg,
  )
    ? value.workingSquareLightSvg
    : DEFAULT_APPEARANCE_SETTINGS.workingSquareLightSvg;
  const workingSquareDarkSvg = isDarkWorkingSquareSvg(
    value.workingSquareDarkSvg,
  )
    ? value.workingSquareDarkSvg
    : DEFAULT_APPEARANCE_SETTINGS.workingSquareDarkSvg;
  const workingSquareLightImageFile = normalizeImageFileName(
    value.workingSquareLightImageFile,
  );
  const workingSquareDarkImageFile = normalizeImageFileName(
    value.workingSquareDarkImageFile,
  );
  const animationSpeed = clampAnimationSpeed(
    typeof value.animationSpeed === "number"
      ? value.animationSpeed
      : DEFAULT_APPEARANCE_SETTINGS.animationSpeed,
  );
  const transparency = clampTransparency(
    typeof value.transparency === "number"
      ? value.transparency
      : DEFAULT_APPEARANCE_SETTINGS.transparency,
  );
  const cardTransparency = clampTransparency(
    typeof value.cardTransparency === "number"
      ? value.cardTransparency
      : DEFAULT_APPEARANCE_SETTINGS.cardTransparency,
    DEFAULT_APPEARANCE_SETTINGS.cardTransparency,
  );
  const textTransparency = clampTransparency(
    typeof value.textTransparency === "number"
      ? value.textTransparency
      : DEFAULT_APPEARANCE_SETTINGS.textTransparency,
    DEFAULT_APPEARANCE_SETTINGS.textTransparency,
  );
  return {
    theme,
    workingSquareLightSvg,
    workingSquareDarkSvg,
    workingSquareLightImageFile,
    workingSquareDarkImageFile,
    motionMode,
    animationSpeed,
    transparency,
    cardTransparency,
    textTransparency,
  };
};

export const parseAppearanceSettings = (rawValue: string | null) => {
  if (!rawValue) return { ...DEFAULT_APPEARANCE_SETTINGS };

  try {
    return normalizeAppearanceSettings(JSON.parse(rawValue));
  } catch {
    return { ...DEFAULT_APPEARANCE_SETTINGS };
  }
};

export const effectiveTheme = (
  themeMode: ThemeMode,
  systemPrefersDark: boolean,
): ResolvedTheme =>
  themeMode === "system" ? (systemPrefersDark ? "dark" : "light") : themeMode;

export const motionIsEnabled = (
  motionMode: MotionMode,
  systemPrefersReducedMotion: boolean,
) => motionMode === "on" || (motionMode === "system" && !systemPrefersReducedMotion);

export const effectiveAnimationSpeed = (
  settings: AppearanceSettings,
  systemPrefersReducedMotion: boolean,
) => {
  if (!motionIsEnabled(settings.motionMode, systemPrefersReducedMotion)) return 0;
  return settings.motionMode === "on" ? settings.animationSpeed : 1;
};
