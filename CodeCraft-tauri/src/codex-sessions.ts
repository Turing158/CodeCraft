import type {
  ClaudePlanRequest,
  ClaudeQuestionRequest,
} from "./claude-sessions";

export type CodexSessionStatus =
  | "working"
  | "waitingForInput"
  | "waitingForApproval"
  | "idle"
  | "stopped";

export type CodexInteractionKind =
  | "permissionsApproval"
  | "userInput"
  | "plan";

export interface CodexActivity {
  id: string;
  tool: string;
  summary: string;
  status: string;
  startedAt: number;
  updatedAt: number;
}

export interface CodexOutputEntry {
  id: string;
  text: string;
}

export interface CodexQuestionOption {
  label: string;
  description: string | null;
}

export interface CodexQuestion {
  id: string;
  header: string;
  question: string;
  options: CodexQuestionOption[];
  isOther: boolean;
  isSecret: boolean;
}

export interface CodexInteraction {
  requestId: string;
  kind: CodexInteractionKind;
  answerable: boolean;
  threadId: string;
  title: string;
  detail: string;
  plan: string | null;
  questions: CodexQuestion[];
  allowSession: boolean;
  isSecret: boolean;
  resolved: boolean;
  capturedAt: number;
}

export interface CodexSession {
  id: string;
  status: CodexSessionStatus;
  title: string;
  cwd: string | null;
  startedAt: number;
  updatedAt: number;
  activities: CodexActivity[];
  outputs: CodexOutputEntry[];
  pendingInteractionId: string | null;
}

export interface CodexSnapshot {
  connected: boolean;
  integrationError: string | null;
  version: number;
  sessions: CodexSession[];
  interactions: CodexInteraction[];
}

export interface CodexPendingInteraction {
  session: CodexSession;
  interaction: CodexInteraction;
}

export type CodexEntryView = "detail" | "approval" | "input" | "plan";

const STATUS_LABELS: Record<CodexSessionStatus, string> = {
  working: "工作中",
  waitingForInput: "等待输入",
  waitingForApproval: "等待审批",
  idle: "空闲",
  stopped: "停止",
};

export function codexStatusLabel(status: CodexSessionStatus): string {
  return STATUS_LABELS[status];
}

export function codexInteractionFor(
  session: CodexSession,
  interactions: CodexInteraction[],
): CodexInteraction | undefined {
  if (!session.pendingInteractionId) return undefined;
  return interactions.find(
    (interaction) => interaction.requestId === session.pendingInteractionId,
  );
}

const interactionMatchesSessionState = (
  session: CodexSession,
  interaction: CodexInteraction,
) =>
  interaction.kind === "permissionsApproval"
    ? session.status === "waitingForApproval"
    : session.status === "waitingForInput";

export function codexPendingInteraction(
  sessions: CodexSession[],
  interactions: CodexInteraction[],
): CodexPendingInteraction | undefined {
  const sessionsById = new Map(sessions.map((session) => [session.id, session]));

  return interactions
    .filter((interaction) => !interaction.resolved)
    .sort((left, right) => right.capturedAt - left.capturedAt)
    .map((interaction) => ({
      interaction,
      session: sessionsById.get(interaction.threadId),
    }))
    .find(
      (entry): entry is CodexPendingInteraction =>
        entry.session !== undefined &&
        entry.session.pendingInteractionId === entry.interaction.requestId &&
        interactionMatchesSessionState(entry.session, entry.interaction),
    );
}

export function codexEntryView(
  session: CodexSession,
  interactions: CodexInteraction[],
): CodexEntryView {
  const interaction = codexInteractionFor(session, interactions);
  if (
    interaction &&
    !interaction.resolved &&
    interactionMatchesSessionState(session, interaction)
  ) {
    if (interaction.kind === "plan") return "plan";
    if (interaction.kind === "userInput") return "input";
    return "approval";
  }
  return "detail";
}

export function codexPlanRequest(
  interaction: CodexInteraction,
  cwd: string | null,
): ClaudePlanRequest {
  return {
    id: interaction.requestId,
    toolName: "Codex Plan",
    plan: interaction.plan ?? interaction.detail,
    cwd,
    capturedAt: interaction.capturedAt,
  };
}

export function codexQuestionRequest(
  interaction: CodexInteraction,
): ClaudeQuestionRequest {
  const externalNotice =
    "此问题来自外部 Codex 会话，请在原终端或 Codex 桌面任务中完成回答。";
  return {
    id: interaction.requestId,
    questions: interaction.questions.map((question) => ({
      header: question.header || "Codex",
      question: `${question.question}\n\n${externalNotice}`,
      options: question.options,
      multiSelect: false,
      allowOther: false,
      allowChat: false,
      readOnly: true,
      answerMode: "外部会话 · 请在原 Codex 界面回答",
    })),
  };
}
