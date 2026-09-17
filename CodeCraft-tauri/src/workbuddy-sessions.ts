export type WorkBuddyRequestKind = "tool" | "question" | "plan";

export type WorkBuddyRequestStatus =
  | "observed"
  | "pending"
  | "completed"
  | "failed"
  | "denied"
  | "cancelled"
  | "timeout"
  | "unavailable"
  | "superseded";

export interface WorkBuddyQuestionOption {
  label: string;
  description: string | null;
}

export interface WorkBuddyQuestion {
  header: string | null;
  question: string;
  options: WorkBuddyQuestionOption[];
  multiSelect: boolean;
  allowOther: boolean;
}

export interface WorkBuddyInteraction {
  requestKey: string;
  sessionId: string;
  kind: WorkBuddyRequestKind;
  status: WorkBuddyRequestStatus;
  nativeRequestId: string | null;
  toolName: string | null;
  toolInput: unknown;
  summary: string;
  questions: WorkBuddyQuestion[];
  plan: string | null;
  planHash?: string | null;
  planSource?: string | null;
  payloadHash: string;
  capturedAt: number;
  expiresAt: number;
  answerable: boolean;
  reason: string;
}

export interface WorkBuddyActivity {
  id: string;
  tool: string;
  summary: string;
  status: string;
  startedAt: number;
  updatedAt: number;
}

export interface WorkBuddyOutput {
  id: string;
  text: string;
}

export interface WorkBuddySession {
  id: string;
  workbuddySessionId?: string;
  title: string;
  titleSource: "native" | "prompt" | "fallback";
  identityStatus?: string;
  source?: string | null;
  workbuddyVersion: string | null;
  cliVersion: string | null;
  pluginInstanceId: string | null;
  processInstanceId: string | null;
  cwdHash: string | null;
  transcriptHash: string | null;
  permissionMode: string | null;
  stage: string;
  currentTool: string | null;
  lastPrompt: string | null;
  eventCount: number;
  startedAt: number;
  updatedAt: number;
  endedAt: number | null;
  pendingCount: number;
  activities: WorkBuddyActivity[];
  outputs: WorkBuddyOutput[];
}

export interface WorkBuddyUnifiedSession extends WorkBuddySession {
  status: ReturnType<typeof workBuddyVisualStatus>;
}

export const toWorkBuddyUnifiedSession = (
  session: WorkBuddySession,
): WorkBuddyUnifiedSession => ({
  ...session,
  title: workBuddySessionTitle(session),
  status: workBuddyVisualStatus(session.stage),
});

export interface WorkBuddyCapabilities {
  canObserve: boolean;
  canApproveTools: boolean;
  canAnswerQuestions: boolean;
  canApprovePlans: boolean;
  canStreamOutput: boolean;
  protocolFrozen: boolean;
  reason: string;
}

export interface WorkBuddyHookState {
  state: "notInstalled" | "installed" | "syncedRestartRequired" | "disabled"
    | "modified" | "conflict" | "incompatible" | "error";
  filesInstalled: boolean;
  enabled: boolean;
  registered: boolean;
  loaded: boolean;
  connected: boolean;
  bridgeReady: boolean;
  error: string | null;
}

export interface WorkBuddySnapshot {
  hook?: WorkBuddyHookState | null;
  connected: boolean;
  integrationError: string | null;
  version: number;
  sessions: WorkBuddySession[];
  interactions: WorkBuddyInteraction[];
  capabilities: WorkBuddyCapabilities;
  observedEventCount: number;
  unknownEventCount: number;
  diagnostics?: {
    duplicateEvents: number;
    lateEvents: number;
    quarantinedEvents: number;
    droppedSessions: number;
    droppedInteractions: number;
    ambiguousCompletions: number;
    retiredProcesses: number;
  };
}

/** All waiting requests, not just the newest one in each session. */
export const workBuddyPendingInteractions = (
  sessions: WorkBuddySession[],
  interactions: WorkBuddyInteraction[],
): { session: WorkBuddySession; interaction: WorkBuddyInteraction }[] => {
  const activeSessions = new Map(sessions
    .filter((session) => session.endedAt === null && session.stage !== "stopped")
    .map((session) => [session.id, session]));
  return interactions.flatMap((interaction) => {
    const session = activeSessions.get(interaction.sessionId);
    const waiting = interaction.status === "pending" || (
      interaction.kind !== "tool" &&
      ["observed", "unavailable"].includes(interaction.status)
    );
    return session && waiting ? [{ session, interaction }] : [];
  }).sort((a, b) => a.interaction.capturedAt - b.interaction.capturedAt ||
    a.interaction.requestKey.localeCompare(b.interaction.requestKey));
};

export const WORKBUDDY_STAGE_LABELS: Record<string, string> = {
  starting: "启动中",
  working: "工作中",
  waitingForInput: "等待输入",
  toolRunning: "调用工具中",
  toolFailed: "工具调用失败",
  idle: "空闲",
  stopped: "已停止",
};

export const workBuddyStageLabel = (stage: string): string =>
  WORKBUDDY_STAGE_LABELS[stage] ?? stage;

export const workBuddyVisualStatus = (stage: string): string => {
  switch (stage) {
    case "working":
    case "toolRunning":
      return "working";
    case "waitingForInput":
      return "waiting";
    case "toolFailed":
      return "toolFailed";
    case "stopped":
      return "stopped";
    default:
      return "idle";
  }
};

export const workBuddyInteractionStatusLabel = (
  status: WorkBuddyRequestStatus,
): string => {
  switch (status) {
    case "pending":
      return "等待审批";
    case "completed":
      return "WorkBuddy 已完成";
    case "failed":
      return "WorkBuddy 执行失败";
    case "denied":
      return "已拒绝";
    case "cancelled":
      return "已取消";
    case "timeout":
      return "已超时";
    case "superseded":
      return "已失效";
    case "unavailable":
      return "只读观察";
    default:
      return "已观察";
  }
};

export const workBuddyRequestKindLabel = (kind: WorkBuddyRequestKind): string => {
  switch (kind) {
    case "question":
      return "问题";
    case "plan":
      return "计划";
    default:
      return "工具";
  }
};

export const workBuddySessionKey = (session: WorkBuddySession): string =>
  `workbuddy:${session.id}`;

const compactWorkBuddyTitle = (value: string | null | undefined): string => {
  const normalized = value?.trim().replace(/\s+/g, " ") ?? "";
  const characters = Array.from(normalized);
  return characters.length <= 96
    ? normalized
    : `${characters.slice(0, 95).join("")}…`;
};

export const workBuddySessionTitle = (session: WorkBuddySession): string =>
  compactWorkBuddyTitle(session.title) ||
  compactWorkBuddyTitle(
    session.lastPrompt?.split(/\r?\n/).find((line) => line.trim()),
  ) ||
  `WorkBuddy 会话 · ${shortWorkBuddySessionId(session.workbuddySessionId || session.id)}`;

const shortWorkBuddySessionId = (sessionId: string): string =>
  sessionId.length <= 24
    ? sessionId
    : `${sessionId.slice(0, 12)}…${sessionId.slice(-8)}`;

export const workBuddyMetadata = (session: WorkBuddySession, connected: boolean): [string, string][] => [
  ["来源", session.source ?? "未知（不推测 Desktop/CLI）"],
  ["Desktop / CLI", `${session.workbuddyVersion ?? "未知"} / ${session.cliVersion ?? "未知"}`],
  ["权限模式", session.permissionMode ?? "未报告"],
  ["工作目录摘要", session.cwdHash ?? "不可用"],
  ["原生会话", session.workbuddySessionId ?? session.id],
  ["集成会话", session.id],
  ["进程实例", session.processInstanceId ?? "未关联"],
  ["插件实例", session.pluginInstanceId ?? "未关联"],
  ["身份状态", session.identityStatus === "observedLineage" ? "已观察进程链（不代表审批授权）" : "身份不完整，仅隔离观察"],
  ["本地桥", connected ? "可用 · 只读" : "连接异常"],
  ["控制通道", "Hook 只读观察"],
];
