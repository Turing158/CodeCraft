import type {
  ClaudePermissionRequest,
  ClaudeQuestionRequest,
} from "./claude-sessions";

export type OpenCodeSessionStatus =
  | "working"
  | "waitingForInput"
  | "waitingForApproval"
  | "toolRunning"
  | "toolFailed"
  | "stopped"
  | "idle";

export interface OpenCodeActivity {
  id: string;
  tool: string;
  summary: string;
  status: string;
  startedAt: number;
  updatedAt: number;
}

export interface OpenCodeOutputEntry {
  id: string;
  text: string;
}

export interface OpenCodeQuestionOption {
  label: string;
  description: string | null;
}

export interface OpenCodeQuestion {
  header: string | null;
  question: string;
  options: OpenCodeQuestionOption[];
  multiSelect: boolean;
  allowOther: boolean;
}

interface OpenCodeReviewBase {
  reviewId: string;
  pluginInstanceId: string;
  sessionId: string;
  callId: string | null;
  capturedAt: number;
  submitting: boolean;
  submissionError?: string | null;
}

export interface OpenCodeQuestionReview extends OpenCodeReviewBase {
  reviewType: "question";
  requestId: string;
  questions: OpenCodeQuestion[];
}

export interface OpenCodeNativePermissionReview extends OpenCodeReviewBase {
  reviewType: "nativePermission";
  requestId: string;
  permission: string;
  patterns: string[];
  always: string[];
  metadata: unknown;
}

export interface OpenCodeStrictToolGateReview extends OpenCodeReviewBase {
  reviewType: "strictToolGate";
  tool: string;
  summary: string;
  ruleKey: string | null;
}

export type OpenCodeReview =
  | OpenCodeQuestionReview
  | OpenCodeNativePermissionReview
  | OpenCodeStrictToolGateReview;

export interface OpenCodeSession {
  id: string;
  pluginInstanceId: string;
  status: OpenCodeSessionStatus;
  title: string;
  cwd: string | null;
  startedAt: number;
  updatedAt: number;
  activities: OpenCodeActivity[];
  outputs: OpenCodeOutputEntry[];
  pendingReviews: OpenCodeReview[];
}

export interface OpenCodeInstance {
  pluginInstanceId: string;
  pluginVersion: string;
  protocolVersion: string;
  processId: number | null;
  directory: string | null;
  worktree: string | null;
  startedAt: number;
  heartbeat: number;
  capabilities?: unknown;
  restartRequired?: boolean;
}

export interface OpenCodeSnapshot {
  connected: boolean;
  integrationError: string | null;
  sessions: OpenCodeSession[];
  instances: OpenCodeInstance[];
}

export interface OpenCodePendingReview {
  session: OpenCodeSession;
  review: OpenCodeReview;
}

const STATUS_LABELS: Record<OpenCodeSessionStatus, string> = {
  working: "工作中",
  waitingForInput: "等待输入",
  waitingForApproval: "等待审批",
  toolRunning: "调用工具中",
  toolFailed: "工具调用失败",
  stopped: "停止",
  idle: "空闲",
};

export const openCodeStatusLabel = (status: OpenCodeSessionStatus): string =>
  STATUS_LABELS[status];

export const openCodeSessionKey = (session: OpenCodeSession): string =>
  `${session.pluginInstanceId}:${session.id}`;

export const openCodeReviewKey = (review: OpenCodeReview): string =>
  `${review.pluginInstanceId}:${review.reviewId}`;

const compareOpenCodeReviews = (left: OpenCodeReview, right: OpenCodeReview) => {
  return left.capturedAt - right.capturedAt;
};

export const openCodeReviewsForSession = (
  session: OpenCodeSession,
): OpenCodeReview[] => [...session.pendingReviews].sort(compareOpenCodeReviews);

export const openCodeReviewForSession = (
  session: OpenCodeSession,
): OpenCodeReview | undefined => openCodeReviewsForSession(session)[0];

export const openCodePendingReview = (
  sessions: OpenCodeSession[],
): OpenCodePendingReview | undefined =>
  sessions
    .flatMap((session) =>
      openCodeReviewsForSession(session).map((review) => ({ session, review })),
    )
    .sort((left, right) => compareOpenCodeReviews(left.review, right.review))[0];

export const openCodePendingReveal = (
  sessions: OpenCodeSession[],
  lastAutoRevealedReviewId?: string,
  dismissedReviewId?: string,
) => {
  const pending = openCodePendingReview(sessions);
  if (!pending) return undefined;
  const reviewId = openCodeReviewKey(pending.review);
  return {
    ...pending,
    reviewId,
    dismissed: reviewId === dismissedReviewId,
    shouldAutoReveal: reviewId !== lastAutoRevealedReviewId,
  };
};

export const openCodeEntryView = (
  session: OpenCodeSession,
): "detail" | "question" | "permission" => {
  const review = openCodeReviewForSession(session);
  if (!review) return "detail";
  if (review.reviewType === "question") return "question";
  return "permission";
};

export const openCodeQuestionRequest = (
  review: OpenCodeQuestionReview,
): ClaudeQuestionRequest => ({
  id: openCodeReviewKey(review),
  questions: review.questions.map((question) => ({
    header: question.header ?? "OpenCode",
    question: question.question,
    options: question.options,
    multiSelect: question.multiSelect,
    allowOther: question.allowOther,
    allowChat: false,
    answerMode: question.multiSelect ? "可多选" : "单选",
  })),
});

const metadataSummary = (metadata: unknown): string | undefined => {
  if (!metadata || typeof metadata !== "object") return undefined;
  try {
    const serialized = JSON.stringify(metadata);
    return serialized === "{}" ? undefined : serialized;
  } catch {
    return undefined;
  }
};

export const openCodePermissionRequest = (
  review: OpenCodeNativePermissionReview | OpenCodeStrictToolGateReview,
  session: OpenCodeSession,
): ClaudePermissionRequest => {
  if (review.reviewType === "strictToolGate") {
    return {
      id: openCodeReviewKey(review),
      toolName: `CodeCraft 全工具门禁 · ${review.tool}`,
      summary: review.summary,
      cwd: session.cwd,
      canAlwaysAllow: review.ruleKey !== null,
      capturedAt: review.capturedAt,
    };
  }

  const details = [
    review.patterns.length > 0 ? review.patterns.join("\n") : undefined,
    metadataSummary(review.metadata),
  ].filter((value): value is string => Boolean(value));
  return {
    id: openCodeReviewKey(review),
    toolName: `OpenCode 原生权限 · ${review.permission}`,
    summary: details.join("\n") || `OpenCode 请求 ${review.permission} 权限`,
    cwd: session.cwd,
    canAlwaysAllow: review.always.length > 0,
    capturedAt: review.capturedAt,
  };
};
