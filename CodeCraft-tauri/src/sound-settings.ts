export type SoundPackId =
  | "note-block"
  | "cat"
  | "bone-block"
  | "experience"
  | "custom";

export type SoundEvent =
  | "toolCall"
  | "permissionApproval"
  | "planExecution"
  | "sessionSuccess"
  | "sessionFailure";

export type CustomSoundFiles = Record<SoundEvent, string | null>;

export interface SoundSettings {
  enabled: boolean;
  volume: number;
  pack: SoundPackId;
  customFiles: CustomSoundFiles;
}

export interface SoundSessionFrame {
  status: string;
  activityIds: string[];
  failedActivityIds: string[];
  permissionId: string | null;
  planId: string | null;
}

export const SOUND_SETTINGS_STORAGE_KEY = "codecraft.sound-settings";
export const MIN_SOUND_VOLUME = 0;
export const MAX_SOUND_VOLUME = 1;

export const SOUND_EVENTS: readonly SoundEvent[] = [
  "toolCall",
  "permissionApproval",
  "planExecution",
  "sessionSuccess",
  "sessionFailure",
];

export const DEFAULT_SOUND_SETTINGS: SoundSettings = {
  enabled: true,
  volume: 0.7,
  pack: "note-block",
  customFiles: {
    toolCall: null,
    permissionApproval: null,
    planExecution: null,
    sessionSuccess: null,
    sessionFailure: null,
  },
};

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null;

const isSoundPack = (value: unknown): value is SoundPackId =>
  value === "note-block" ||
  value === "cat" ||
  value === "bone-block" ||
  value === "experience" ||
  value === "custom";

export const clampSoundVolume = (value: number) => {
  if (!Number.isFinite(value)) return DEFAULT_SOUND_SETTINGS.volume;
  return Math.min(MAX_SOUND_VOLUME, Math.max(MIN_SOUND_VOLUME, value));
};

const normalizeCustomFiles = (value: unknown): CustomSoundFiles => {
  const source = isRecord(value) ? value : {};
  const files = {} as CustomSoundFiles;
  for (const event of SOUND_EVENTS) {
    const fileName = source[event];
    files[event] = typeof fileName === "string" && fileName.trim() ? fileName : null;
  }
  return files;
};

export const normalizeSoundSettings = (value: unknown): SoundSettings => {
  if (!isRecord(value)) return cloneDefaultSoundSettings();
  return {
    enabled: value.enabled !== false,
    volume: clampSoundVolume(
      typeof value.volume === "number"
        ? value.volume
        : DEFAULT_SOUND_SETTINGS.volume,
    ),
    pack: isSoundPack(value.pack) ? value.pack : DEFAULT_SOUND_SETTINGS.pack,
    customFiles: normalizeCustomFiles(value.customFiles),
  };
};

export const parseSoundSettings = (rawValue: string | null): SoundSettings => {
  if (!rawValue) return cloneDefaultSoundSettings();
  try {
    return normalizeSoundSettings(JSON.parse(rawValue));
  } catch {
    return cloneDefaultSoundSettings();
  }
};

export const cloneDefaultSoundSettings = (): SoundSettings => ({
  ...DEFAULT_SOUND_SETTINGS,
  customFiles: { ...DEFAULT_SOUND_SETTINGS.customFiles },
});

const hasNewValue = (previous: string | null | undefined, next: string | null) =>
  next !== null && next !== previous;

const hasNewIds = (
  previous: readonly string[] | undefined,
  next: readonly string[],
) => {
  const previousIds = new Set(previous ?? []);
  return next.some((id) => !previousIds.has(id));
};

/**
 * Compares two polling frames and returns one notification per meaningful
 * transition. The first frame for a newly observed session can be passed with
 * an undefined previous value; callers should skip the initial application-wide
 * snapshot to avoid replaying old events on startup.
 */
export const soundEventsForTransition = (
  previous: SoundSessionFrame | undefined,
  next: SoundSessionFrame,
): SoundEvent[] => {
  const events: SoundEvent[] = [];
  if (hasNewIds(previous?.activityIds, next.activityIds)) {
    events.push("toolCall");
  }
  if (hasNewValue(previous?.permissionId, next.permissionId)) {
    events.push("permissionApproval");
  }
  if (hasNewValue(previous?.planId, next.planId)) {
    events.push("planExecution");
  }

  const failedActivityAdded = hasNewIds(
    previous?.failedActivityIds,
    next.failedActivityIds,
  );
  const sessionFailed =
    next.status === "toolFailed" && previous?.status !== "toolFailed";
  if (failedActivityAdded || sessionFailed) {
    events.push("sessionFailure");
  } else if (
    (next.status === "stopped" || next.status === "idle") &&
    previous?.status !== next.status &&
    previous?.status !== undefined &&
    previous.status !== "toolFailed"
  ) {
    events.push("sessionSuccess");
  }
  return events;
};
