import type {
  ClaudePermissionRequest,
  ClaudePlanRequest,
  ClaudeQuestionRequest,
} from "./claude-sessions";

export type DshSessionStatus =
  | "working"
  | "waitingForInput"
  | "waitingForApproval"
  | "toolRunning"
  | "toolFailed"
  | "stopped"
  | "idle";

export interface DshQuestionOption {
  label: string;
  description: string | null;
}

export interface DshQuestion {
  id: string;
  header: string | null;
  question: string;
  detail: string | null;
  options: DshQuestionOption[];
  multiSelect: boolean;
  intent: { kind: "plan-review"; approve: string } | null;
}

export interface DshQuestionRequest {
  id: string;
  bridgeInstanceId: string;
  pluginInstanceId: string;
  sessionId: string;
  questions: DshQuestion[];
  capturedAt: number;
}

export interface DshPermissionRequest extends ClaudePermissionRequest {
  bridgeInstanceId: string;
  pluginInstanceId: string;
  sessionId: string;
  callId: string | null;
}

export interface DshPlanRequest extends ClaudePlanRequest {
  bridgeInstanceId: string;
  pluginInstanceId: string;
  sessionId: string;
  approveLabel: string;
  declineLabel: string;
}

export interface DshActivity {
  id: string;
  tool: string;
  summary: string;
  status: string;
  startedAt: number;
  updatedAt: number;
}

export interface DshSession {
  id: string;
  pluginInstanceId: string;
  bridgeInstanceId: string;
  status: DshSessionStatus;
  title: string;
  cwd: string | null;
  startedAt: number;
  updatedAt: number;
  activities: DshActivity[];
  outputs: Array<{ id: string; text: string }>;
  question: DshQuestionRequest | null;
  permission: DshPermissionRequest | null;
  plan: DshPlanRequest | null;
}

export interface DshInstance {
  pluginInstanceId: string;
  pluginVersion: string;
  dshVersion: string;
  dshProcessId: number;
  bridgeInstanceId: string;
  workspace: string | null;
  heartbeat: number;
  capabilities: {
    observation: boolean;
    toolApproval: boolean;
    questionAnswer: boolean;
    planReview: boolean;
    allowAlways: boolean;
  };
}

export interface DshSnapshot {
  connected: boolean;
  integrationError: string | null;
  bridgeInstanceId: string | null;
  sessions: DshSession[];
  instances: DshInstance[];
}

export type DshReviewSourceCapabilities = {
  canRespond: true;
  canAllowOnce: true;
  canAlwaysAllow: false;
  canDeny: true;
  canApprovePlan: true;
  canKeepPlanning: true;
  readOnly: false;
};

export const DSH_REVIEW_CAPABILITIES: DshReviewSourceCapabilities = {
  canRespond: true,
  canAllowOnce: true,
  canAlwaysAllow: false,
  canDeny: true,
  canApprovePlan: true,
  canKeepPlanning: true,
  readOnly: false,
};

export const dshSessionKey = (session: DshSession): string =>
  `dsh:${session.pluginInstanceId}:${session.id}`;

const STATUS_LABELS: Record<DshSessionStatus, string> = {
  working: "工作中",
  waitingForInput: "等待输入",
  waitingForApproval: "等待审批",
  toolRunning: "调用工具中",
  toolFailed: "工具调用失败",
  stopped: "停止",
  idle: "空闲",
};

export const dshStatusLabel = (status: DshSessionStatus): string =>
  STATUS_LABELS[status];

export const dshQuestionForUi = (
  request: DshQuestionRequest,
): ClaudeQuestionRequest => ({
  id: request.id,
  questions: request.questions.map((question) => ({
    header: question.header,
    question: question.detail
      ? `${question.question}\n\n${question.detail}`
      : question.question,
    options: question.options,
    multiSelect: question.multiSelect,
    allowOther: true,
    allowChat: false,
  })),
});

export type DshPendingReview =
  | { view: "plan"; session: DshSession; request: DshPlanRequest }
  | { view: "permission"; session: DshSession; request: DshPermissionRequest }
  | { view: "question"; session: DshSession; request: DshQuestionRequest };

export const dshPendingReview = (
  sessions: DshSession[],
): DshPendingReview | undefined => {
  const session = sessions
    .filter((candidate) => candidate.plan || candidate.permission || candidate.question)
    .sort((left, right) => right.updatedAt - left.updatedAt)[0];
  if (!session) return undefined;
  if (session.plan) return { view: "plan", session, request: session.plan };
  if (session.permission)
    return { view: "permission", session, request: session.permission };
  if (session.question)
    return { view: "question", session, request: session.question };
  return undefined;
};
