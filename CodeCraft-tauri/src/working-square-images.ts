export type WorkingSquareImageTheme = "light" | "dark";
export type WorkingSquareImageTarget = WorkingSquareImageTheme | "system";

export const workingSquareImageThemeForTarget = (
  target: string | undefined,
  systemPrefersDark: boolean,
): WorkingSquareImageTheme | undefined => {
  if (target === "light" || target === "dark") return target;
  if (target === "system") return systemPrefersDark ? "dark" : "light";
  return undefined;
};

export const MAX_WORKING_SQUARE_IMAGE_BYTES = 10 * 1024 * 1024;

export type WorkingSquareImageValidation =
  | "valid"
  | "unsupported"
  | "too-large";

type ImageFileDescriptor = Pick<File, "name" | "size" | "type">;

const IMAGE_EXTENSION_PATTERN =
  /\.(?:avif|bmp|gif|ico|jpe?g|png|svg|webp)$/i;

export const validateWorkingSquareImageFile = (
  file: ImageFileDescriptor,
): WorkingSquareImageValidation => {
  if (file.size > MAX_WORKING_SQUARE_IMAGE_BYTES) return "too-large";
  if (file.type.toLowerCase().startsWith("image/")) return "valid";
  return IMAGE_EXTENSION_PATTERN.test(file.name) ? "valid" : "unsupported";
};

export const workingSquareImageIsDecodable = (file: Blob) =>
  new Promise<boolean>((resolve) => {
    if (typeof Image === "undefined") {
      resolve(true);
      return;
    }

    const url = URL.createObjectURL(file);
    const image = new Image();
    let timeout: number | undefined;
    const finish = (result: boolean) => {
      if (timeout !== undefined) window.clearTimeout(timeout);
      image.onload = null;
      image.onerror = null;
      URL.revokeObjectURL(url);
      resolve(result);
    };
    timeout = window.setTimeout(() => finish(false), 5000);
    image.onload = () => finish(image.naturalWidth > 0 && image.naturalHeight > 0);
    image.onerror = () => finish(false);
    image.src = url;
  });

const DATABASE_NAME = "codecraft-working-square-images";
const STORE_NAME = "files";

const objectUrls = new Map<WorkingSquareImageTheme, string>();
let databasePromise: Promise<IDBDatabase | null> | undefined;

const openDatabase = (): Promise<IDBDatabase | null> => {
  if (databasePromise) return databasePromise;
  databasePromise = new Promise((resolve) => {
    if (typeof indexedDB === "undefined") {
      resolve(null);
      return;
    }
    const request = indexedDB.open(DATABASE_NAME, 1);
    request.onupgradeneeded = () => {
      request.result.createObjectStore(STORE_NAME);
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => resolve(null);
  });
  return databasePromise;
};

const saveImageFile = async (theme: WorkingSquareImageTheme, file: File) => {
  const database = await openDatabase();
  if (!database) return false;
  return new Promise<boolean>((resolve) => {
    try {
      const transaction = database.transaction(STORE_NAME, "readwrite");
      transaction.objectStore(STORE_NAME).put(file, theme);
      transaction.oncomplete = () => resolve(true);
      transaction.onerror = () => resolve(false);
      transaction.onabort = () => resolve(false);
    } catch {
      resolve(false);
    }
  });
};

const readImageFile = async (
  theme: WorkingSquareImageTheme,
): Promise<Blob | null> => {
  const database = await openDatabase();
  if (!database) return null;
  return new Promise((resolve) => {
    try {
      const request = database
        .transaction(STORE_NAME, "readonly")
        .objectStore(STORE_NAME)
        .get(theme);
      request.onsuccess = () =>
        resolve(request.result instanceof Blob ? request.result : null);
      request.onerror = () => resolve(null);
    } catch {
      resolve(null);
    }
  });
};

const deleteImageFile = async (theme: WorkingSquareImageTheme) => {
  const database = await openDatabase();
  if (!database) return;
  await new Promise<void>((resolve) => {
    try {
      const transaction = database.transaction(STORE_NAME, "readwrite");
      transaction.objectStore(STORE_NAME).delete(theme);
      transaction.oncomplete = () => resolve();
      transaction.onerror = () => resolve();
      transaction.onabort = () => resolve();
    } catch {
      resolve();
    }
  });
};

const setObjectUrl = (theme: WorkingSquareImageTheme, file: Blob) => {
  const previousUrl = objectUrls.get(theme);
  if (previousUrl) URL.revokeObjectURL(previousUrl);
  const url = URL.createObjectURL(file);
  objectUrls.set(theme, url);
  return url;
};

export const createWorkingSquareImageStore = () => ({
  urlFor: (theme: WorkingSquareImageTheme) => objectUrls.get(theme),
  load: async (theme: WorkingSquareImageTheme) => {
    const file = await readImageFile(theme);
    if (!file || objectUrls.has(theme)) return objectUrls.get(theme);
    return setObjectUrl(theme, file);
  },
  save: async (theme: WorkingSquareImageTheme, file: File) => {
    const saved = await saveImageFile(theme, file);
    if (!saved) throw new Error("Unable to persist working square image");
    return setObjectUrl(theme, file);
  },
  remove: async (theme: WorkingSquareImageTheme) => {
    await deleteImageFile(theme);
    const previousUrl = objectUrls.get(theme);
    if (previousUrl) URL.revokeObjectURL(previousUrl);
    objectUrls.delete(theme);
  },
});
