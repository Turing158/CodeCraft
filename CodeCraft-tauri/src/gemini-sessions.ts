export type GeminiSessionStatus =
  | "working"
  | "waitingForInput"
  | "toolRunning"
  | "toolCompleted"
  | "toolFailed"
  | "idle"
  | "stopped";

export type GeminiInteractionKind =
  | "askUser"
  | "toolPermission"
  | "fileChange"
  | "shell"
  | "mcp"
  | "sandboxExpansion"
  | "exitPlanMode"
  | "notification";

export type GeminiObservationStatus =
  | "observed"
  | "updated"
  | "toolCompleted"
  | "sessionEnded"
  | "stale"
  | "navigationAvailable"
  | "navigationUnavailable";

export interface GeminiActivity {
  id: string;
  tool: string;
  summary: string;
  status: string;
  startedAt: number;
  updatedAt: number;
}

export interface GeminiInteraction {
  observationId: string;
  interactionKey: string;
  kind: GeminiInteractionKind;
  status: GeminiObservationStatus;
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

export interface GeminiTerminalBinding {
  pid: number | null;
  parentPid: number | null;
  processCreatedAt: number | null;
  consoleWindow: string | null;
  capturedAt: number;
}

export interface GeminiSession {
  id: string;
  title: string;
  cwd: string | null;
  status: GeminiSessionStatus;
  startedAt: number;
  updatedAt: number;
  endedAt: number | null;
  activities: GeminiActivity[];
  outputs: Array<{ id: string; text: string }>;
  pendingInteractions: GeminiInteraction[];
  terminalBinding: GeminiTerminalBinding | null;
  integrationStatus: string;
}

export interface GeminiSnapshot {
  connected: boolean;
  integrationError: string | null;
  version: number;
  sessions: GeminiSession[];
  interactions: GeminiInteraction[];
  navigationCapability: string;
  fallbackAction: "openGeminiOnHost" | string;
}

export const GEMINI_REVIEW_CAPABILITIES = {
  canRespond: false,
  canAllowOnce: false,
  canAlwaysAllow: false,
  canDeny: false,
  canApprovePlan: false,
  canKeepPlanning: false,
  readOnly: true,
} as const;

export const geminiSessionKey = (session: GeminiSession): string =>
  `gemini:${session.id}`;

const STATUS_LABELS: Record<GeminiSessionStatus, string> = {
  working: "工作中",
  waitingForInput: "等待 Gemini 输入",
  toolRunning: "调用工具中",
  toolCompleted: "工具已完成",
  toolFailed: "工具失败",
  idle: "空闲",
  stopped: "已停止",
};

export const geminiStatusLabel = (status: GeminiSessionStatus): string =>
  STATUS_LABELS[status] ?? status;

export const geminiInteractionFor = (
  session: GeminiSession,
): GeminiInteraction | undefined =>
  [...session.pendingInteractions]
    .filter(
      (interaction) =>
        interaction.status !== "toolCompleted" &&
        interaction.status !== "sessionEnded",
    )
    .sort((left, right) => left.capturedAt - right.capturedAt)[0];

export const geminiSessionStatusLabel = (session: GeminiSession): string =>
  geminiStatusLabel(session.status);
