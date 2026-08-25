/**
 * Update check logic for the About settings card.
 *
 * The latest release tag is taken from the first element of
 * `https://api.github.com/repos/Turing158/CodeCraft/releases?per_page=1`,
 * its leading "v" is stripped, and the result is compared against the
 * installed version as plain strings — version ordering is intentionally
 * ignored, only equality matters.
 */

export type UpdateCheckState = "download" | "upToDate" | "noInfo";

/** Strips exactly one leading lowercase "v" from a release tag. */
export const normalizeReleaseTag = (tagName: string): string =>
  tagName.startsWith("v") ? tagName.slice(1) : tagName;

/**
 * Resolves which button the About card should show.
 *
 * - `download`: a release tag exists and differs from the installed version.
 * - `upToDate`: the release tag equals the installed version.
 * - `noInfo`: there is no usable release information.
 */
export const resolveUpdateCheck = (
  installedVersion: string,
  latestTag: string | undefined | null,
): UpdateCheckState => {
  if (typeof latestTag !== "string" || latestTag.trim() === "") return "noInfo";
  const latestVersion = normalizeReleaseTag(latestTag);
  if (latestVersion === "") return "noInfo";
  return latestVersion === installedVersion ? "upToDate" : "download";
};

export const UP_TO_DATE_HOLD_MS = 5 * 60 * 1000;

/**
 * Coarse global lifecycle state shown in the About settings panel.
 * "checking" and "downloading" drive the loading animation, while
 * "download" and "upToDate" keep their resolved result visible until
 * "upToDate" reverts to "idle" after UP_TO_DATE_HOLD_MS.
 */
export type UpdateLifecycle =
  | "idle"
  | "checking"
  | "downloading"
  | "download"
  | "upToDate";

/** Maps a resolved check state onto the global lifecycle. */
export const resolvedLifecycle = (
  state: UpdateCheckState,
): UpdateLifecycle => {
  if (state === "download") return "download";
  if (state === "upToDate") return "upToDate";
  return "idle";
};