import type {
  ClaudePermissionRequest,
  ClaudeQuestionRequest,
} from "./claude-sessions";

export type PiSessionStatus =
  | "working"
  | "waitingForInput"
  | "waitingForApproval"
  | "toolRunning"
  | "stopped"
  | "idle";

export interface PiPermissionRequest extends ClaudePermissionRequest {
  extensionInstanceId: string;
  sessionId: string;
  runId: string | null;
}

export interface PiQuestionRequest extends ClaudeQuestionRequest {
  extensionInstanceId: string;
  sessionId: string;
}

export interface PiActivity {
  id: string;
  tool: string;
  summary: string;
  status: string;
  startedAt: number;
  updatedAt: number;
}

export interface PiSession {
  id: string;
  installId: string;
  extensionInstanceId: string;
  status: PiSessionStatus;
  title: string;
  cwd: string | null;
  startedAt: number;
  updatedAt: number;
  activities: PiActivity[];
  outputs: Array<{ id: string; text: string }>;
  question: PiQuestionRequest | null;
  permission: PiPermissionRequest | null;
}

export interface PiInstance {
  extensionInstanceId: string;
  installId: string;
  piVersion: string | null;
  protocolVersion: string;
  heartbeat: number;
  capabilities?: unknown;
}

export interface PiSnapshot {
  connected: boolean;
  integrationError: string | null;
  version?: number;
  sessions: PiSession[];
  instances: PiInstance[];
}

export interface PiPendingPermission {
  session: PiSession;
  request: PiPermissionRequest;
}

export const piPendingPermission = (
  sessions: PiSession[],
): PiPendingPermission | undefined => {
  const pending = sessions
    .filter((session) => session.permission !== null)
    .sort((left, right) => right.updatedAt - left.updatedAt)[0];
  return pending?.permission
    ? { session: pending, request: pending.permission }
    : undefined;
};

export type PiSessionEntryView =
  | "detail"
  | "question"
  | "permission";

export const piSessionKey = (session: PiSession): string =>
  `pi:${session.installId}:${session.id}`;

const PI_STATUS_LABELS: Record<PiSessionStatus, string> = {
  working: "工作中",
  waitingForInput: "等待输入",
  waitingForApproval: "等待审批",
  toolRunning: "调用工具中",
  stopped: "停止",
  idle: "空闲",
};

export const piStatusLabel = (status: PiSessionStatus): string =>
  PI_STATUS_LABELS[status];

export const piSessionEntryView = (session: PiSession): PiSessionEntryView => {
  if (session.permission) return "permission";
  if (session.question) return "question";
  return "detail";
};

export interface PiPendingReview {
  session: PiSession;
  view: Exclude<PiSessionEntryView, "detail">;
  request: PiPermissionRequest | PiQuestionRequest;
}

export const piPendingReview = (
  sessions: PiSession[],
): PiPendingReview | undefined => {
  const session = sessions
    .filter(
      (candidate) =>
        candidate.permission !== null ||
        candidate.question !== null,
    )
    .sort((left, right) => right.updatedAt - left.updatedAt)[0];
  if (!session) return undefined;
  const view = piSessionEntryView(session);
  if (view === "permission" && session.permission) {
    return { session, view, request: session.permission };
  }
  if (view === "question" && session.question) {
    return { session, view, request: session.question };
  }
  return undefined;
};
