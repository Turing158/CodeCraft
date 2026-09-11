import type { ClaudePermissionRequest, ClaudePlanRequest, ClaudeQuestionRequest } from "./claude-sessions";

export type KimiSessionStatus =
  | "working"
  | "waitingForInput"
  | "toolRunning"
  | "toolCompleted"
  | "toolFailed"
  | "idle"
  | "stopped";

export type KimiInteractionKind =
  | "askUser"
  | "toolPermission"
  | "fileChange"
  | "shell"
  | "mcp"
  | "sandboxExpansion"
  | "exitPlanMode"
  | "notification";

export type KimiObservationStatus =
  | "observed"
  | "updated"
  | "toolCompleted"
  | "sessionEnded"
  | "stale"
  | "navigationAvailable"
  | "navigationUnavailable";

export interface KimiActivity {
  id: string;
  tool: string;
  summary: string;
  status: string;
  startedAt: number;
  updatedAt: number;
}

export interface KimiInteraction {
  observationId: string;
  interactionKey: string;
  nativeInteractionId?: string | null;
  wireSessionId?: string | null;
  agentId?: string | null;
  resolved?: boolean;
  kind: KimiInteractionKind;
  status: KimiObservationStatus;
  title: string;
  detail: string;
  toolName: string | null;
  toolCallId: string | null;
  toolInput: unknown | null;
  questions: unknown[];
  planFilename: string | null;
  plan: string | null;
  planReadError: string | null;
  capturedAt: number;
  truncated: boolean;
  navigationAvailable: boolean;
}

export interface KimiTerminalBinding {
  pid: number | null;
  parentPid: number | null;
  processCreatedAt: number | null;
  consoleWindow: string | null;
  windowProcessId?: number | null;
  windowProcessCreatedAt?: number | null;
  sharedTerminal?: boolean;
  capturedAt: number;
}

export interface KimiSession {
  id: string;
  kimiSessionId: string | null;
  integrationSessionKey: string;
  hookInstallId: string | null;
  clientType: string | null;
  processInstanceId?: string | null;
  kimiVersion?: string | null;
  title: string;
  cwd: string | null;
  status: KimiSessionStatus;
  startedAt: number;
  updatedAt: number;
  endedAt: number | null;
  activities: KimiActivity[];
  outputs: Array<{ id: string; text: string }>;
  pendingInteractions: KimiInteraction[];
  terminalBinding: KimiTerminalBinding | null;
  integrationStatus: string;
}

export interface KimiSnapshot {
  connected: boolean;
  integrationError: string | null;
  version: number;
  sessions: KimiSession[];
  interactions: KimiInteraction[];
  quarantinedEvents?: number;
  droppedEvents?: number;
  navigationCapability: string;
  fallbackAction: "openKimiOnHost" | string;
  capabilities: {
    canObserve: boolean;
    canApproveTools: boolean;
    canAnswerQuestions: boolean;
    canApprovePlans: boolean;
    canStreamOutput: boolean;
    readOnly: boolean;
    reason: string;
  };
}

export const KIMI_REVIEW_CAPABILITIES = {
  canRespond: false,
  canAllowOnce: false,
  canAlwaysAllow: false,
  canDeny: false,
  canApprovePlan: false,
  canKeepPlanning: false,
  readOnly: true,
} as const;

export const kimiSessionKey = (session: KimiSession): string =>
  `kimi:${session.id}`;

const STATUS_LABELS: Record<KimiSessionStatus, string> = {
  working: "工作中",
  waitingForInput: "等待 Kimi 输入",
  toolRunning: "调用工具中",
  toolCompleted: "工具已完成",
  toolFailed: "工具失败",
  idle: "空闲",
  stopped: "已停止",
};

export const kimiStatusLabel = (status: KimiSessionStatus): string =>
  STATUS_LABELS[status] ?? status;

export const kimiInteractionFor = (
  session: KimiSession,
): KimiInteraction | undefined =>
  (session.status === "idle" || session.status === "stopped" ? [] : [...session.pendingInteractions])
    .filter(
      (interaction) =>
        interaction.kind !== "notification" &&
        interaction.resolved !== true &&
        (interaction.status === "observed" || interaction.status === "updated"),
    )
    .sort((left, right) => left.capturedAt - right.capturedAt)[0];

export const kimiSessionStatusLabel = (session: KimiSession): string =>
  kimiStatusLabel(session.status);

export type KimiReview =
  | { kind: "question"; request: ClaudeQuestionRequest }
  | { kind: "permission"; request: ClaudePermissionRequest }
  | { kind: "plan"; request: ClaudePlanRequest };

const record = (value: unknown): Record<string, unknown> =>
  typeof value === "object" && value !== null ? value as Record<string, unknown> : {};

export const kimiReviewFor = (session: KimiSession): KimiReview | undefined => {
  const interaction = kimiInteractionFor(session);
  if (!interaction) return undefined;
  const id = interaction.interactionKey;
  if (interaction.kind === "askUser") {
    return { kind: "question", request: {
      id,
      questions: (interaction.questions.length ? interaction.questions : [interaction.detail]).map((raw) => {
        const question = record(raw);
        return {
          header: typeof question.header === "string" ? question.header : interaction.title,
          question: typeof raw === "string" ? raw : typeof question.question === "string" ? question.question : interaction.detail,
          options: Array.isArray(question.options) ? question.options.flatMap((rawOption) => {
            const option = record(rawOption);
            const label = typeof rawOption === "string" ? rawOption : option.label;
            return typeof label === "string" ? [{ label, description: typeof option.description === "string" ? option.description : null }] : [];
          }) : [],
          multiSelect: question.multiSelect === true || question.multi_select === true,
          allowOther: false,
          allowChat: false,
          readOnly: true,
          answerMode: "Kimi Code",
        };
      }),
    } };
  }
  if (interaction.kind === "exitPlanMode") {
    const text = interaction.plan ?? (interaction.planFilename ? `计划文件：${interaction.planFilename}` : interaction.detail);
    return { kind: "plan", request: {
      id,
      toolName: interaction.title,
      plan: [text, interaction.planReadError].filter(Boolean).join("\n\n"),
      cwd: session.cwd,
      capturedAt: interaction.capturedAt,
    } };
  }
  return { kind: "permission", request: {
    id,
    toolName: interaction.toolName ?? interaction.title,
    summary: interaction.detail,
    cwd: session.cwd,
    canAlwaysAllow: false,
    capturedAt: interaction.capturedAt,
  } };
};

