export const SETTINGS_PLACEMENT_STORAGE_KEY =
  "codecraft.lan.settings-placement";

export const SETTINGS_PLACEMENTS = [
  "left",
  "right",
  "top",
  "bottom",
] as const;

export type SettingsPlacement = (typeof SETTINGS_PLACEMENTS)[number];

export const DEFAULT_SETTINGS_PLACEMENT: SettingsPlacement = "bottom";

export const isSettingsPlacement = (
  value: unknown,
): value is SettingsPlacement =>
  typeof value === "string" &&
  SETTINGS_PLACEMENTS.includes(value as SettingsPlacement);

export const loadSettingsPlacement = (
  storage: Pick<Storage, "getItem">,
): SettingsPlacement => {
  try {
    const value = storage.getItem(SETTINGS_PLACEMENT_STORAGE_KEY);
    return isSettingsPlacement(value) ? value : DEFAULT_SETTINGS_PLACEMENT;
  } catch {
    return DEFAULT_SETTINGS_PLACEMENT;
  }
};

export const saveSettingsPlacement = (
  storage: Pick<Storage, "setItem">,
  placement: SettingsPlacement,
): void => {
  try {
    storage.setItem(SETTINGS_PLACEMENT_STORAGE_KEY, placement);
  } catch {
    // The setting still applies to the current page load.
  }
};
