import type {
  ClaudePermissionRequest,
  ClaudePlanRequest,
  ClaudeQuestionRequest,
} from "./claude-sessions";

export type MimoSessionStatus =
  | "working"
  | "waitingForInput"
  | "waitingForApproval"
  | "toolRunning"
  | "toolFailed"
  | "stopped"
  | "idle";

export interface MimoActivity {
  id: string;
  tool: string;
  summary: string;
  status: string;
  startedAt: number;
  updatedAt: number;
}

export interface MimoOutputEntry {
  id: string;
  text: string;
}

export interface MimoQuestionOption {
  label: string;
  description: string | null;
}

export interface MimoQuestion {
  header: string | null;
  question: string;
  options: MimoQuestionOption[];
  multiSelect: boolean;
  allowOther: boolean;
}

interface MimoReviewBase {
  reviewId: string;
  pluginInstanceId: string;
  sessionId: string;
  callId: string | null;
  capturedAt: number;
  submitting: boolean;
  submissionError?: string | null;
}

export interface MimoQuestionReview extends MimoReviewBase {
  reviewType: "question";
  requestId: string;
  questions: MimoQuestion[];
}

export interface MimoNativePermissionReview extends MimoReviewBase {
  reviewType: "nativePermission";
  requestId: string;
  permission: string;
  patterns: string[];
  always: string[];
  metadata: unknown;
}

export interface MimoStrictToolGateReview extends MimoReviewBase {
  reviewType: "strictToolGate";
  tool: string;
  summary: string;
  ruleKey: string | null;
}

export interface MimoPlanReview extends MimoReviewBase {
  reviewType: "plan";
  requestId: string;
  questions: MimoQuestion[];
  planPath: string | null;
  planBody: string | null;
}

export type MimoReview =
  | MimoQuestionReview
  | MimoNativePermissionReview
  | MimoStrictToolGateReview
  | MimoPlanReview;

export interface MimoSession {
  source: "mimo";
  id: string;
  pluginInstanceId: string;
  status: MimoSessionStatus;
  title: string;
  cwd: string | null;
  startedAt: number;
  updatedAt: number;
  activities: MimoActivity[];
  outputs: MimoOutputEntry[];
  pendingReviews: MimoReview[];
}

export interface MimoInstance {
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

export interface MimoSnapshot {
  connected: boolean;
  integrationError: string | null;
  version?: number;
  sessions: MimoSession[];
  instances: MimoInstance[];
}

export interface MimoPendingReview {
  session: MimoSession;
  review: MimoReview;
}

const STATUS_LABELS: Record<MimoSessionStatus, string> = {
  working: "工作中",
  waitingForInput: "等待输入",
  waitingForApproval: "等待审批",
  toolRunning: "调用工具中",
  toolFailed: "工具调用失败",
  stopped: "停止",
  idle: "空闲",
};

export const mimoStatusLabel = (status: MimoSessionStatus): string =>
  STATUS_LABELS[status];

export const mimoSessionKey = (session: MimoSession): string =>
  `${session.pluginInstanceId}:${session.id}`;

export const mimoReviewKey = (review: MimoReview): string =>
  `${review.pluginInstanceId}:${review.reviewId}`;

const compareMimoReviews = (left: MimoReview, right: MimoReview) => {
  return left.capturedAt - right.capturedAt;
};

export const mimoReviewsForSession = (
  session: MimoSession,
): MimoReview[] => [...session.pendingReviews].sort(compareMimoReviews);

export const mimoReviewForSession = (
  session: MimoSession,
): MimoReview | undefined => mimoReviewsForSession(session)[0];

export const mimoPendingReview = (
  sessions: MimoSession[],
): MimoPendingReview | undefined =>
  sessions
    .flatMap((session) =>
      mimoReviewsForSession(session).map((review) => ({ session, review })),
    )
    .sort((left, right) => compareMimoReviews(left.review, right.review))[0];

export const mimoPendingReveal = (
  sessions: MimoSession[],
  lastAutoRevealedReviewId?: string,
  dismissedReviewId?: string,
) => {
  const pending = mimoPendingReview(sessions);
  if (!pending) return undefined;
  const reviewId = mimoReviewKey(pending.review);
  return {
    ...pending,
    reviewId,
    dismissed: reviewId === dismissedReviewId,
    shouldAutoReveal: reviewId !== lastAutoRevealedReviewId,
  };
};

export const mimoEntryView = (
  session: MimoSession,
): "detail" | "question" | "permission" | "plan" => {
  const review = mimoReviewForSession(session);
  if (!review) return "detail";
  if (review.reviewType === "plan") return "plan";
  if (review.reviewType === "question") return "question";
  return "permission";
};

export const mimoQuestionRequest = (
  review: MimoQuestionReview,
): ClaudeQuestionRequest => ({
  id: mimoReviewKey(review),
  questions: review.questions.map((question) => ({
    header: question.header ?? "Mimo",
    question: question.question,
    options: question.options,
    multiSelect: question.multiSelect,
    allowOther: question.allowOther,
    allowChat: false,
    answerMode: question.multiSelect ? "可多选" : "单选",
  })),
});

export const mimoPlanRequest = (
  review: MimoPlanReview,
  session: MimoSession,
): ClaudePlanRequest => ({
  id: mimoReviewKey(review),
  toolName: "Mimo 计划审批",
  plan: review.planBody ?? review.questions[0]?.question ?? "Mimo 请求确认计划",
  cwd: session.cwd,
  capturedAt: review.capturedAt,
});

const metadataSummary = (
  metadata: unknown,
  permission: string,
): string | undefined => {
  if (!metadata || typeof metadata !== "object") return undefined;

  const record = metadata as Record<string, unknown>;
  const args =
    record.args && typeof record.args === "object"
      ? (record.args as Record<string, unknown>)
      : record;
  const stringValue = (...keys: string[]): string | undefined => {
    for (const key of keys) {
      const value = args[key];
      if (typeof value === "string" && value.trim()) return value;
    }
    return undefined;
  };

  const filePath = stringValue(
    "file_path",
    "filePath",
    "notebook_path",
    "notebookPath",
  );
  if (filePath) {
    const normalizedPermission = permission.trim().toLowerCase();
    const action =
      normalizedPermission === "read"
        ? "读取文件"
        : normalizedPermission === "write"
          ? "写入文件"
          : normalizedPermission === "edit"
            ? "编辑文件"
            : "访问文件";
    return `${action}：${filePath}`;
  }

  const command = stringValue("command", "cmd", "script");
  if (command) return `执行命令：${command}`;

  const description = stringValue("description", "reason", "message");
  if (description) return description;

  const path = stringValue("path", "directory", "dir");
  if (path) return `访问路径：${path}`;

  const pattern = stringValue("pattern", "query");
  if (pattern) return `匹配内容：${pattern}`;

  const url = stringValue("url");
  if (url) return `访问网址：${url}`;

  try {
    const entries = Object.entries(args).filter(([, value]) =>
      ["string", "number", "boolean"].includes(typeof value),
    );
    if (entries.length > 0) {
      return entries
        .map(([key, value]) => `参数 ${key}：${String(value)}`)
        .join("\n");
    }
  } catch {
    return undefined;
  }

  return undefined;
};

export const mimoPermissionRequest = (
  review: MimoNativePermissionReview | MimoStrictToolGateReview,
  session: MimoSession,
): ClaudePermissionRequest => {
  if (review.reviewType === "strictToolGate") {
    return {
      id: mimoReviewKey(review),
      toolName: `CodeCraft 全工具门禁 · ${review.tool}`,
      summary: review.summary,
      cwd: session.cwd,
      canAlwaysAllow: review.ruleKey !== null,
      capturedAt: review.capturedAt,
    };
  }

  const details = [
    review.patterns.length > 0 ? review.patterns.join("\n") : undefined,
    metadataSummary(review.metadata, review.permission),
  ].filter((value): value is string => Boolean(value));
  return {
    id: mimoReviewKey(review),
    toolName: `Mimo 原生权限 · ${review.permission}`,
    summary: details.join("\n") || `Mimo 请求 ${review.permission} 权限`,
    cwd: session.cwd,
    canAlwaysAllow: review.always.length > 0,
    capturedAt: review.capturedAt,
  };
};
