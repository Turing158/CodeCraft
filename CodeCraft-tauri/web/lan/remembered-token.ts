/** Persists a LAN access token only when the user explicitly opts in. */

export const REMEMBERED_TOKEN_STORAGE_KEY = "codecraft.lan.remembered-token";

type TokenStorage = Pick<Storage, "getItem" | "removeItem" | "setItem">;

export const loadRememberedToken = (storage: TokenStorage): string => {
  try {
    return storage.getItem(REMEMBERED_TOKEN_STORAGE_KEY)?.trim() ?? "";
  } catch {
    return "";
  }
};

export const saveRememberedToken = (
  storage: TokenStorage,
  token: string,
): void => {
  try {
    const value = token.trim();
    if (value) storage.setItem(REMEMBERED_TOKEN_STORAGE_KEY, value);
    else storage.removeItem(REMEMBERED_TOKEN_STORAGE_KEY);
  } catch {
    // The current authenticated session does not depend on browser storage.
  }
};

export const clearRememberedToken = (storage: TokenStorage): void => {
  try {
    storage.removeItem(REMEMBERED_TOKEN_STORAGE_KEY);
  } catch {
    // Storage can be unavailable in private or restricted browser contexts.
  }
};
