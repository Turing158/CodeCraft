import {
  sessionEntryView,
  sessionStatusLabel,
  type ClaudePermissionRequest,
  type ClaudePlanRequest,
  type ClaudeQuestionRequest,
  type ClaudeSession,
  type ClaudeSessionSnapshot,
} from "../../src/claude-sessions";
import {
  codexEntryView,
  codexInteractionFor,
  codexPlanRequest,
  codexQuestionRequest,
  codexStatusLabel,
  type CodexInteraction,
  type CodexSession,
  type CodexSnapshot,
} from "../../src/codex-sessions";

export type ConsoleSource = "claude" | "codex";

export type PendingKind = "permission" | "question" | "plan";

export interface ConsoleActivity {
  id: string;
  tool: string;
  summary: string;
  status: string;
  updatedAt: number;
}

export interface ConsoleOutput {
  id: string;
  text: string;
}

/** A request waiting on a person. Read-only entries can only be viewed. */
export interface ConsolePending {
  kind: PendingKind;
  requestId: string;
  readOnly: boolean;
  permission?: ClaudePermissionRequest;
  question?: ClaudeQuestionRequest;
  plan?: ClaudePlanRequest;
}

export interface ConsoleEntry {
  key: string;
  source: ConsoleSource;
  sessionId: string;
  title: string;
  status: string;
  statusLabel: string;
  cwd: string | null;
  updatedAt: number;
  activities: ConsoleActivity[];
  outputs: ConsoleOutput[];
  pending: ConsolePending | null;
}

export interface ConsoleIntegration {
  source: ConsoleSource;
  name: string;
  connected: boolean;
  error: string | null;
  sessionCount: number;
}

export interface ConsoleSnapshot {
  generatedAt: number;
  allowApprovals: boolean;
  entries: ConsoleEntry[];
  integrations: ConsoleIntegration[];
}

export interface RawSnapshot {
  generatedAt?: number;
  allowApprovals?: boolean;
  claude?: ClaudeSessionSnapshot | null;
  codex?: CodexSnapshot | null;
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null;

const emptyClaude: ClaudeSessionSnapshot = {
  connected: false,
  integrationError: null,
  sessions: [],
};

const emptyCodex: CodexSnapshot = {
  connected: false,
  integrationError: null,
  version: 0,
  sessions: [],
  interactions: [],
};

// Codex questions and plans arrive from an external terminal, so the console can
// display them but cannot answer them.
const codexPending = (
  session: CodexSession,
  interactions: CodexInteraction[],
): ConsolePending | null => {
  const interaction = codexInteractionFor(session, interactions);
  if (!interaction || interaction.resolved) return null;

  switch (codexEntryView(session, interactions)) {
    case "plan":
      return {
        kind: "plan",
        requestId: interaction.requestId,
        readOnly: true,
        plan: codexPlanRequest(interaction, session.cwd),
      };
    case "input":
      return {
        kind: "question",
        requestId: interaction.requestId,
        readOnly: true,
        question: codexQuestionRequest(interaction),
      };
    case "approval":
      return {
        kind: "permission",
        requestId: interaction.requestId,
        readOnly: false,
        permission: {
          id: interaction.requestId,
          toolName: interaction.title || "Codex Hook 工具调用",
          summary: interaction.detail || "Codex Hook 请求执行操作",
          cwd: session.cwd,
          canAlwaysAllow: interaction.allowSession,
          capturedAt: interaction.capturedAt,
        },
      };
    default:
      return null;
  }
};

const claudePending = (session: ClaudeSession): ConsolePending | null => {
  switch (sessionEntryView(session)) {
    case "plan":
      return session.plan
        ? {
            kind: "plan",
            requestId: session.plan.id,
            readOnly: false,
            plan: session.plan,
          }
        : null;
    case "permission":
      return session.permission
        ? {
            kind: "permission",
            requestId: session.permission.id,
            readOnly: false,
            permission: session.permission,
          }
        : null;
    case "question":
      return session.question
        ? {
            kind: "question",
            requestId: session.question.id,
            readOnly: session.question.questions.every(
              (question) => question.readOnly === true,
            ),
            question: session.question,
          }
        : null;
    default:
      return null;
  }
};

const claudeEntry = (session: ClaudeSession): ConsoleEntry => ({
  key: "claude:" + session.id,
  source: "claude",
  sessionId: session.id,
  title: session.title,
  status: session.status,
  statusLabel: sessionStatusLabel(session.status),
  cwd: null,
  updatedAt: session.updatedAt,
  activities: session.activities.map((activity) => ({
    id: activity.id,
    tool: activity.tool,
    summary: activity.summary,
    status: activity.status,
    updatedAt: activity.updatedAt,
  })),
  outputs: session.outputs.map((output) => ({
    id: output.id,
    text: output.text,
  })),
  pending: claudePending(session),
});

const codexEntry = (
  session: CodexSession,
  interactions: CodexInteraction[],
): ConsoleEntry => ({
  key: "codex:" + session.id,
  source: "codex",
  sessionId: session.id,
  title: session.title,
  status: session.status,
  statusLabel: codexStatusLabel(session.status),
  cwd: session.cwd,
  updatedAt: session.updatedAt,
  activities: session.activities.map((activity) => ({
    id: activity.id,
    tool: activity.tool,
    summary: activity.summary,
    status: activity.status,
    updatedAt: activity.updatedAt,
  })),
  outputs: session.outputs.map((output) => ({
    id: output.id,
    text: output.text,
  })),
  pending: codexPending(session, interactions),
});

/** Waiting work first, then the most recently updated session. */
export const compareEntries = (
  left: ConsoleEntry,
  right: ConsoleEntry,
): number => {
  const leftPending = left.pending ? 0 : 1;
  const rightPending = right.pending ? 0 : 1;
  if (leftPending !== rightPending) return leftPending - rightPending;
  if (left.updatedAt !== right.updatedAt) return right.updatedAt - left.updatedAt;
  return left.key.localeCompare(right.key);
};

export const mergeSnapshot = (raw: RawSnapshot): ConsoleSnapshot => {
  const claude = isRecord(raw.claude)
    ? { ...emptyClaude, ...(raw.claude as ClaudeSessionSnapshot) }
    : emptyClaude;
  const codex = isRecord(raw.codex)
    ? { ...emptyCodex, ...(raw.codex as CodexSnapshot) }
    : emptyCodex;

  const entries = [
    ...claude.sessions.map(claudeEntry),
    ...codex.sessions.map((session) => codexEntry(session, codex.interactions)),
  ].sort(compareEntries);

  return {
    generatedAt: typeof raw.generatedAt === "number" ? raw.generatedAt : 0,
    allowApprovals: raw.allowApprovals === true,
    entries,
    integrations: [
      {
        source: "claude",
        name: "Claude Code",
        connected: claude.connected,
        error: claude.integrationError ?? null,
        sessionCount: claude.sessions.length,
      },
      {
        source: "codex",
        name: "Codex",
        connected: codex.connected,
        error: codex.integrationError ?? null,
        sessionCount: codex.sessions.length,
      },
    ],
  };
};

export const pendingCount = (snapshot: ConsoleSnapshot): number =>
  snapshot.entries.filter((entry) => entry.pending !== null).length;

export const findEntry = (
  snapshot: ConsoleSnapshot,
  key: string | undefined,
): ConsoleEntry | undefined =>
  key ? snapshot.entries.find((entry) => entry.key === key) : undefined;

/** Picks the request a person should look at first, used for auto-reveal. */
export const firstPendingEntry = (
  snapshot: ConsoleSnapshot,
): ConsoleEntry | undefined =>
  snapshot.entries.find((entry) => entry.pending !== null);

const PENDING_LABELS: Record<PendingKind, string> = {
  permission: "等待审批",
  question: "等待回答",
  plan: "等待确认计划",
};

export const pendingLabel = (pending: ConsolePending): string =>
  pending.readOnly
    ? PENDING_LABELS[pending.kind] + " · 只读"
    : PENDING_LABELS[pending.kind];

/**
 * Whether the console may submit this request: remote approvals must be on and
 * the request must not be one of the read-only Codex kinds.
 */
export const canAct = (
  pending: ConsolePending | null,
  allowApprovals: boolean,
): boolean => pending !== null && !pending.readOnly && allowApprovals;
