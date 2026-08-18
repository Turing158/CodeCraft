export type ContentView =
  | "sessions"
  | "detail"
  | "settings"
  | "question"
  | "permission"
  | "plan";

export type ContentTransitionDirection = "forward" | "backward" | "neutral";

export type ReviewContentView = Extract<
  ContentView,
  "question" | "permission" | "plan"
>;

export const shouldRestoreReviewOrigin = (
  reviewView: ReviewContentView,
  currentView: ContentView,
): boolean => reviewView === currentView;

export const captureReviewOrigin = (
  reviewView: ReviewContentView,
  currentView: ContentView,
  existingOrigin?: ContentView,
): ContentView | undefined =>
  reviewView === currentView ? existingOrigin : currentView;

export const reviewReturnView = (
  origin: ContentView | undefined,
  hasSelectedSession: boolean,
): ContentView => origin ?? (hasSelectedSession ? "detail" : "sessions");

export const contentTransitionDirection = (
  from: ContentView,
  to: ContentView,
): ContentTransitionDirection => {
  if (from === "sessions" && to === "detail") return "forward";
  if (from === "detail" && to === "sessions") return "backward";
  if (from === "sessions" && to === "settings") return "forward";
  if (from === "settings" && to === "sessions") return "backward";
  if (from !== "question" && to === "question") return "forward";
  if (from === "question" && to !== "question") return "backward";
  if (from !== "plan" && to === "plan") return "forward";
  if (from === "plan" && to !== "plan") return "backward";
  if (from !== "permission" && to === "permission") return "forward";
  if (from === "permission" && to !== "permission") return "backward";
  return "neutral";
};
