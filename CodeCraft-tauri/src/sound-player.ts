import approvalBoneBlockUrl from "../src-tauri/sound/approval-bone-block.ogg?url";
import approvalCatUrl from "../src-tauri/sound/approval-cat.ogg?url";
import approvalExperienceUrl from "../src-tauri/sound/approval-experience.ogg?url";
import approvalNoteBlockUrl from "../src-tauri/sound/approval-note-block.ogg?url";
import taskCompleteBoneBlockUrl from "../src-tauri/sound/task-complete-bone-block.ogg?url";
import taskCompleteCatUrl from "../src-tauri/sound/task-complete-cat.ogg?url";
import taskCompleteExperienceUrl from "../src-tauri/sound/task-complete-experience.ogg?url";
import taskCompleteNoteBlockUrl from "../src-tauri/sound/task-complete-note-block.ogg?url";
import type { SoundEvent, SoundPackId, SoundSettings } from "./sound-settings";

type PresetSoundPair = {
  approval: string;
  taskComplete: string;
};

export type PresetPreviewKind = keyof PresetSoundPair;

const presetSounds: Record<Exclude<SoundPackId, "custom">, PresetSoundPair> = {
  "note-block": {
    approval: approvalNoteBlockUrl,
    taskComplete: taskCompleteNoteBlockUrl,
  },
  cat: { approval: approvalCatUrl, taskComplete: taskCompleteCatUrl },
  "bone-block": {
    approval: approvalBoneBlockUrl,
    taskComplete: taskCompleteBoneBlockUrl,
  },
  experience: {
    approval: approvalExperienceUrl,
    taskComplete: taskCompleteExperienceUrl,
  },
};

const CUSTOM_SOUND_DB = "codecraft-sounds";
const CUSTOM_SOUND_STORE = "files";
const customSoundUrls = new Map<SoundEvent, string>();
let databasePromise: Promise<IDBDatabase | null> | undefined;

const openDatabase = (): Promise<IDBDatabase | null> => {
  if (databasePromise) return databasePromise;
  databasePromise = new Promise((resolve) => {
    if (typeof indexedDB === "undefined") {
      resolve(null);
      return;
    }
    const request = indexedDB.open(CUSTOM_SOUND_DB, 1);
    request.onupgradeneeded = () => {
      request.result.createObjectStore(CUSTOM_SOUND_STORE);
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => resolve(null);
  });
  return databasePromise;
};

const saveCustomFile = async (event: SoundEvent, file: File) => {
  const database = await openDatabase();
  if (!database) return;
  await new Promise<void>((resolve) => {
    const transaction = database.transaction(CUSTOM_SOUND_STORE, "readwrite");
    transaction.objectStore(CUSTOM_SOUND_STORE).put(file, event);
    transaction.oncomplete = () => resolve();
    transaction.onerror = () => resolve();
    transaction.onabort = () => resolve();
  });
};

const readCustomFile = async (event: SoundEvent): Promise<Blob | null> => {
  const database = await openDatabase();
  if (!database) return null;
  return new Promise((resolve) => {
    const request = database
      .transaction(CUSTOM_SOUND_STORE, "readonly")
      .objectStore(CUSTOM_SOUND_STORE)
      .get(event);
    request.onsuccess = () => resolve(request.result instanceof Blob ? request.result : null);
    request.onerror = () => resolve(null);
  });
};

const playUrl = (url: string, volume: number) => {
  const audio = new Audio(url);
  audio.volume = volume;
  void audio.play().catch(() => undefined);
};

export const createSoundPlayer = (getSettings: () => SoundSettings) => {
  const customUrlFor = async (event: SoundEvent) => {
    let url = customSoundUrls.get(event);
    if (!url && getSettings().customFiles[event]) {
      const file = await readCustomFile(event);
      if (file) {
        url = URL.createObjectURL(file);
        customSoundUrls.set(event, url);
      }
    }
    return url;
  };

  const playEvent = async (event: SoundEvent, force = false) => {
    const settings = getSettings();
    if (!force && !settings.enabled) return;
    if (settings.pack === "custom") {
      const url = await customUrlFor(event);
      if (url) playUrl(url, settings.volume);
      return;
    }
    const pair = presetSounds[settings.pack];
    playUrl(
      event === "sessionSuccess" || event === "sessionFailure"
        ? pair.taskComplete
        : pair.approval,
      settings.volume,
    );
  };

  return {
    playEvent,
    previewPack: (
      pack: Exclude<SoundPackId, "custom">,
      kind: PresetPreviewKind = "approval",
    ) => {
      const pair = presetSounds[pack];
      playUrl(pair[kind], getSettings().volume);
    },
    previewCustom: async (event: SoundEvent) => {
      const url = await customUrlFor(event);
      if (url) playUrl(url, getSettings().volume);
    },
    customFile: (event: SoundEvent) => readCustomFile(event),
    saveCustomFile: async (event: SoundEvent, file: File) => {
      const previousUrl = customSoundUrls.get(event);
      if (previousUrl) URL.revokeObjectURL(previousUrl);
      const url = URL.createObjectURL(file);
      customSoundUrls.set(event, url);
      await saveCustomFile(event, file);
      return file.name;
    },
  };
};
