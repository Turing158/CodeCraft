/** Layout decisions for the LAN console, kept separate so they can be tested
 * without a browser. */

export type Breakpoint = "phone" | "tablet" | "desktop";

export const TABLET_MIN_WIDTH = 600;
export const DESKTOP_MIN_WIDTH = 960;

export const breakpointFor = (width: number): Breakpoint => {
  if (!Number.isFinite(width) || width < TABLET_MIN_WIDTH) return "phone";
  if (width < DESKTOP_MIN_WIDTH) return "tablet";
  return "desktop";
};

/** Desktop shows the list and the detail side by side; the narrower layouts
 * show one at a time and present detail as a sliding sheet. */
export const showsBothPanes = (breakpoint: Breakpoint): boolean =>
  breakpoint === "desktop";

export const detailIsSheet = (breakpoint: Breakpoint): boolean =>
  breakpoint === "phone";

/** A wide two-pane layout should never open with an unexplained blank detail
 * column when sessions are already available. */
export const shouldAutoSelectFirst = (
  width: number,
  hasSelection: boolean,
  entryCount: number,
): boolean =>
  showsBothPanes(breakpointFor(width)) && !hasSelection && entryCount > 0;

export type ConsoleView = "list" | "detail";

export interface LayoutDecision {
  breakpoint: Breakpoint;
  bothPanes: boolean;
  asSheet: boolean;
  /** The pane that owns the screen when only one can be shown. */
  visiblePane: ConsoleView;
}

export const layoutFor = (
  width: number,
  view: ConsoleView,
  hasSelection: boolean,
): LayoutDecision => {
  const breakpoint = breakpointFor(width);
  const bothPanes = showsBothPanes(breakpoint);
  const wantsDetail = view === "detail" && hasSelection;
  return {
    breakpoint,
    bothPanes,
    asSheet: detailIsSheet(breakpoint) && wantsDetail,
    visiblePane: bothPanes || !wantsDetail ? "list" : "detail",
  };
};

const RELATIVE_TIME_UNITS: Array<[number, string]> = [
  [1000, "刚刚"],
  [60_000, "秒"],
  [3_600_000, "分钟"],
];

/** Turns a snapshot timestamp into the freshness label in the status bar. */
export const freshnessLabel = (generatedAt: number, now: number): string => {
  if (!generatedAt) return "尚无数据";
  const elapsed = Math.max(0, now - generatedAt);
  if (elapsed < RELATIVE_TIME_UNITS[0][0]) return "刚刚更新";
  if (elapsed < RELATIVE_TIME_UNITS[1][0]) {
    return Math.floor(elapsed / 1000) + " 秒前";
  }
  if (elapsed < RELATIVE_TIME_UNITS[2][0]) {
    return Math.floor(elapsed / 60_000) + " 分钟前";
  }
  return Math.floor(elapsed / 3_600_000) + " 小时前";
};
