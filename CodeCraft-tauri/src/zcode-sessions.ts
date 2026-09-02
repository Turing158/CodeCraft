import type {
  ClaudePermissionRequest,
  ClaudePlanRequest,
  ClaudeQuestionRequest,
} from "./claude-sessions";
import type { LocalQuestionAnswer } from "./question-flow";

export type ZCodeSessionStatus =
  | "working"
  | "waitingForInput"
  | "waitingForApproval"
  | "toolRunning"
  | "toolFailed"
  | "stopped"
  | "idle";

export type ZCodeReviewState =
  | "pending"
  | "submitted"
  | "returnedToZCode";

export interface ZCodeQuestionOption {
  label: string;
  description: string | null;
  preview?: unknown;
}

export interface ZCodeQuestion {
  header: string | null;
  question: string;
  options: ZCodeQuestionOption[];
  multiSelect: boolean;
  allowOther: boolean;
  allowChat: boolean;
}

export interface ZCodeQuestionRequest extends ClaudeQuestionRequest {
  sessionId: string;
  capturedAt: number;
  questions: ZCodeQuestion[];
}

export interface ZCodePermissionRequest extends ClaudePermissionRequest {
  sessionId: string;
  canAlwaysAllow: false;
}

export interface ZCodePlanRequest extends ClaudePlanRequest {
  sessionId: string;
  allowedPrompts: unknown | null;
}

export interface ZCodeActivity {
  id: string;
  tool: string;
  summary: string;
  status: string;
  startedAt: number;
  updatedAt: number;
}

export interface ZCodeSession {
  id: string;
  status: ZCodeSessionStatus;
  title: string;
  cwd: string | null;
  startedAt: number;
  updatedAt: number;
  activities: ZCodeActivity[];
  outputs: Array<{ id: string; text: string }>;
  question: ZCodeQuestionRequest | null;
  permission: ZCodePermissionRequest | null;
  plan: ZCodePlanRequest | null;
  reviewState: ZCodeReviewState | null;
}

export interface ZCodeCapabilities {
  observation: boolean;
  toolApproval: boolean;
  questionAnswer: boolean;
  planReview: boolean;
  planFeedback: boolean;
  allowAlways: false;
  streamingAnswer: false;
  questionSound: false;
}

export interface ZCodeSnapshot {
  connected: boolean;
  integrationError: string | null;
  detectedPath: string | null;
  detectedVersion: string | null;
  capabilities: ZCodeCapabilities;
  sessions: ZCodeSession[];
}

export type ZCodeReviewSourceCapabilities = {
  canRespond: true;
  canAllowOnce: true;
  canAlwaysAllow: false;
  canDeny: true;
  canApprovePlan: true;
  canKeepPlanning: true;
  readOnly: false;
};

export const ZCODE_REVIEW_CAPABILITIES: ZCodeReviewSourceCapabilities = {
  canRespond: true,
  canAllowOnce: true,
  canAlwaysAllow: false,
  canDeny: true,
  canApprovePlan: true,
  canKeepPlanning: true,
  readOnly: false,
};

export const zcodeSessionKey = (session: ZCodeSession): string =>
  `zcode:${session.id}`;

const STATUS_LABELS: Record<ZCodeSessionStatus, string> = {
  working: "工作中",
  waitingForInput: "等待回答",
  waitingForApproval: "等待审批",
  toolRunning: "调用工具中",
  toolFailed: "工具调用失败",
  stopped: "本轮已停止",
  idle: "空闲",
};

export const zcodeStatusLabel = (status: ZCodeSessionStatus): string =>
  STATUS_LABELS[status];

export const zcodeSessionStatusLabel = (session: ZCodeSession): string => {
  if (session.reviewState === "returnedToZCode") return "已交回 ZCode";
  if (session.reviewState === "submitted") return "决定已提交";
  return zcodeStatusLabel(session.status);
};

export const zcodeQuestionForUi = (
  request: ZCodeQuestionRequest,
): ClaudeQuestionRequest => ({
  id: request.id,
  questions: request.questions.map((question) => ({
    header: question.header,
    question: question.question,
    options: question.options.map(({ label, description, preview }) => ({
      label,
      description,
      preview,
    })),
    multiSelect: question.multiSelect,
    allowOther: question.allowOther,
    allowChat: false,
  })),
});

export const normalizeZCodeQuestionAnswers = (
  answers: LocalQuestionAnswer[],
): LocalQuestionAnswer[] =>
  answers.map((answer) => ({
    ...answer,
    selectedOptionLabels: answer.selectedOptionLabels.filter(
      (label) => label !== "其他",
    ),
    extraText: answer.extraText?.trim() || null,
  }));

export const zcodeQuestionAnswersAreValid = (
  answers: LocalQuestionAnswer[],
): boolean =>
  normalizeZCodeQuestionAnswers(answers).every(
    (answer) =>
      answer.selectedOptionLabels.some((label) => label.trim().length > 0) ||
      Boolean(answer.extraText),
  );

export type ZCodePendingReview =
  | { view: "plan"; session: ZCodeSession; request: ZCodePlanRequest }
  | {
      view: "permission";
      session: ZCodeSession;
      request: ZCodePermissionRequest;
    }
  | {
      view: "question";
      session: ZCodeSession;
      request: ZCodeQuestionRequest;
    };

export const zcodePendingReview = (
  sessions: ZCodeSession[],
): ZCodePendingReview | undefined => {
  const session = sessions
    .filter((candidate) => candidate.plan || candidate.permission || candidate.question)
    .sort((left, right) => right.updatedAt - left.updatedAt)[0];
  if (!session) return undefined;
  if (session.plan) return { view: "plan", session, request: session.plan };
  if (session.permission) {
    return { view: "permission", session, request: session.permission };
  }
  if (session.question) {
    return { view: "question", session, request: session.question };
  }
  return undefined;
};
