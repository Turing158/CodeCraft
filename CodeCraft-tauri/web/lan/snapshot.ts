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
import {
  openCodePermissionRequest,
  openCodeQuestionRequest,
  openCodeReviewKey,
  openCodeStatusLabel,
  type OpenCodeReview,
  type OpenCodeSession,
  type OpenCodeSnapshot,
} from "../../src/opencode-sessions";
import {
  mimoPermissionRequest,
  mimoPlanRequest,
  mimoQuestionRequest,
  mimoReviewKey,
  mimoStatusLabel,
  type MimoReview,
  type MimoSession,
  type MimoSnapshot,
} from "../../src/mimo-sessions";
import {
  piSessionEntryView,
  piSessionKey,
  piStatusLabel,
  type PiSession,
  type PiSnapshot,
} from "../../src/pi-sessions";
import {
  dshQuestionForUi,
  dshSessionKey,
  dshStatusLabel,
  type DshSession,
  type DshSnapshot,
} from "../../src/dsh-sessions";
import {
  zcodeQuestionForUi,
  zcodeSessionKey,
  zcodeSessionStatusLabel,
  type ZCodeSession,
  type ZCodeSnapshot,
} from "../../src/zcode-sessions";
import {
  geminiSessionKey,
  geminiStatusLabel,
  type GeminiSession,
  type GeminiSnapshot,
} from "../../src/gemini-sessions";
import {
  kimiSessionKey,
  kimiStatusLabel,
  type KimiSession,
  type KimiSnapshot,
} from "../../src/kimi-sessions";

export type ConsoleSource =
  | "claude"
  | "codex"
  | "opencode"
  | "mimo"
  | "pi"
  | "dsh"
  | "zcode"
  | "gemini"
  | "kimi";

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
  openCode?: {
    pluginInstanceId: string;
    sessionId: string;
    reviewId: string;
    requestId?: string;
    reviewType: OpenCodeReview["reviewType"];
  };
  pi?: {
    extensionInstanceId: string;
    sessionId: string;
  };
  mimo?: {
    pluginInstanceId: string;
    sessionId: string;
    reviewId: string;
    requestId?: string;
    reviewType: MimoReview["reviewType"];
  };
  dsh?: {
    bridgeInstanceId: string;
    pluginInstanceId: string;
    sessionId: string;
  };
  zcode?: {
    sessionId: string;
  };
  gemini?: {
    sessionId: string;
  };
  kimi?: {
    sessionId: string;
  };
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
  /** Whether the agent tool itself is present on the LAN machine. */
  agentInstalled: boolean;
  /** Whether the CodeCraft hook is installed for this agent. */
  hookInstalled: boolean;
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
  opencode?: OpenCodeSnapshot | null;
  mimo?: MimoSnapshot | null;
  pi?: PiSnapshot | null;
  dsh?: DshSnapshot | null;
  zcode?: ZCodeSnapshot | null;
  gemini?: GeminiSnapshot | null;
  kimi?: KimiSnapshot | null;
  integrations?: RawHookIntegration[] | null;
}

/** Per-agent install state, mirroring the desktop hook settings list. */
export interface RawHookIntegration {
  id?: string;
  name?: string;
  agentInstalled?: boolean;
  hookInstalled?: boolean;
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

const emptyOpenCode: OpenCodeSnapshot = {
  connected: false,
  integrationError: null,
  sessions: [],
  instances: [],
};

const emptyPi: PiSnapshot = {
  connected: false,
  integrationError: null,
  sessions: [],
  instances: [],
};

const emptyMimo: MimoSnapshot = {
  connected: false,
  integrationError: null,
  sessions: [],
  instances: [],
};

const emptyDsh: DshSnapshot = {
  connected: false,
  integrationError: null,
  bridgeInstanceId: null,
  sessions: [],
  instances: [],
};

const emptyZCode: ZCodeSnapshot = {
  connected: false,
  integrationError: null,
  detectedPath: null,
  detectedVersion: null,
  capabilities: {
    observation: true,
    toolApproval: true,
    questionAnswer: true,
    planReview: true,
    planFeedback: true,
    allowAlways: false,
    streamingAnswer: false,
    questionSound: false,
  },
  sessions: [],
};
const emptyGemini: GeminiSnapshot = {
  connected: false,
  integrationError: null,
  version: 0,
  sessions: [],
  interactions: [],
  navigationCapability: "unsupported",
  fallbackAction: "openGeminiOnHost",
};
const emptyKimi: KimiSnapshot = {
  connected: false,
  integrationError: null,
  version: 0,
  sessions: [],
  interactions: [],
  navigationCapability: "unsupported",
  fallbackAction: "openKimiOnHost",
  capabilities: {
    canObserve: true,
    canApproveTools: false,
    canAnswerQuestions: false,
    canApprovePlans: false,
    canStreamOutput: false,
    readOnly: true,
    reason: "Kimi Code Hook 已连接",
  },
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

const openCodePending = (
  session: OpenCodeSession,
): ConsolePending | null => {
  const review = [...session.pendingReviews].sort(
    (left, right) => left.capturedAt - right.capturedAt,
  )[0];
  if (!review) return null;
  const target = {
    pluginInstanceId: review.pluginInstanceId,
    sessionId: review.sessionId,
    reviewId: review.reviewId,
    requestId: "requestId" in review ? review.requestId : undefined,
    reviewType: review.reviewType,
  };
  if (review.reviewType === "question") {
    return {
      kind: "question",
      requestId: openCodeReviewKey(review),
      readOnly: false,
      question: openCodeQuestionRequest(review),
      openCode: target,
    };
  }
  return {
    kind: "permission",
    requestId: openCodeReviewKey(review),
    readOnly: false,
    permission: openCodePermissionRequest(review, session),
    openCode: target,
  };
};

const piPending = (session: PiSession): ConsolePending | null => {
  const target = {
    extensionInstanceId: session.extensionInstanceId,
    sessionId: session.id,
  };
  switch (piSessionEntryView(session)) {
    case "permission":
      return session.permission
        ? {
            kind: "permission",
            requestId: session.permission.id,
            readOnly: false,
            permission: session.permission,
            pi: target,
          }
        : null;
    case "question":
      return session.question
        ? {
            kind: "question",
            requestId: session.question.id,
            readOnly: false,
            question: session.question,
            pi: target,
          }
        : null;
    default:
      return null;
  }
};

const mimoPending = (session: MimoSession): ConsolePending | null => {
  const review = [...session.pendingReviews].sort((left, right) => left.capturedAt - right.capturedAt)[0];
  if (!review) return null;
  const target = {
    pluginInstanceId: review.pluginInstanceId,
    sessionId: review.sessionId,
    reviewId: review.reviewId,
    requestId: "requestId" in review ? review.requestId : undefined,
    reviewType: review.reviewType,
  };
  if (review.reviewType === "plan") {
    return { kind: "plan", requestId: mimoReviewKey(review), readOnly: false, plan: mimoPlanRequest(review, session), mimo: target };
  }
  if (review.reviewType === "question") {
    return { kind: "question", requestId: mimoReviewKey(review), readOnly: false, question: mimoQuestionRequest(review), mimo: target };
  }
  return { kind: "permission", requestId: mimoReviewKey(review), readOnly: false, permission: mimoPermissionRequest(review, session), mimo: target };
};

const dshPending = (session: DshSession): ConsolePending | null => {
  const target = {
    bridgeInstanceId: session.bridgeInstanceId,
    pluginInstanceId: session.pluginInstanceId,
    sessionId: session.id,
  };
  if (session.plan) {
    return {
      kind: "plan",
      requestId: session.plan.id,
      readOnly: false,
      plan: session.plan,
      dsh: target,
    };
  }
  if (session.permission) {
    return {
      kind: "permission",
      requestId: session.permission.id,
      readOnly: false,
      permission: session.permission,
      dsh: target,
    };
  }
  if (session.question) {
    return {
      kind: "question",
      requestId: session.question.id,
      readOnly: false,
      question: dshQuestionForUi(session.question),
      dsh: target,
    };
  }
  return null;
};

const zcodePending = (session: ZCodeSession): ConsolePending | null => {
  const target = { sessionId: session.id };
  if (session.plan) {
    return {
      kind: "plan",
      requestId: session.plan.id,
      readOnly: false,
      plan: session.plan,
      zcode: target,
    };
  }
  if (session.permission) {
    return {
      kind: "permission",
      requestId: session.permission.id,
      readOnly: false,
      permission: session.permission,
      zcode: target,
    };
  }
  if (session.question) {
    return {
      kind: "question",
      requestId: session.question.id,
      readOnly: false,
      question: zcodeQuestionForUi(session.question),
      zcode: target,
    };
  }
  return null;
};

const geminiPending = (session: GeminiSession): ConsolePending | null => {
  const interaction = session.pendingInteractions
    .filter((item) => item.status !== "toolCompleted" && item.status !== "sessionEnded")
    .sort((left, right) => left.capturedAt - right.capturedAt)[0];
  if (!interaction) return null;
  const common = { requestId: interaction.observationId, readOnly: true };
  if (interaction.kind === "askUser") {
    return {
      ...common,
      kind: "question",
      question: {
        id: interaction.observationId,
        questions: (interaction.questions.length ? interaction.questions : [{ question: interaction.detail }]).map((raw) => {
          const item = isRecord(raw) ? raw : {};
          return {
            header: typeof item.header === "string" ? item.header : interaction.title,
            question: typeof item.question === "string" ? item.question : interaction.detail,
            options: Array.isArray(item.options) ? item.options.flatMap((option) => isRecord(option) && typeof option.label === "string" ? [{ label: option.label, description: typeof option.description === "string" ? option.description : null }] : []) : [],
            multiSelect: item.multiSelect === true,
            allowOther: false,
            allowChat: false,
            readOnly: true,
          };
        }),
      },
      gemini: { sessionId: session.id },
    };
  }
  if (interaction.kind === "exitPlanMode") {
    const planText = interaction.plan
      ?? (interaction.planFilename
        ? `计划文件：${interaction.planFilename}`
        : interaction.detail);
    return {
      ...common,
      kind: "plan",
      plan: {
        id: interaction.observationId,
        toolName: interaction.title,
        plan: interaction.planReadError
          ? `${planText}\n\n计划文件不可读取：${interaction.planReadError}`
          : planText,
        cwd: session.cwd,
        capturedAt: interaction.capturedAt,
      },
      gemini: { sessionId: session.id },
    };
  }
  return {
    ...common,
    kind: "permission",
    permission: {
      id: interaction.observationId,
      toolName: interaction.toolName ?? interaction.title,
      summary: interaction.detail,
      cwd: session.cwd,
      canAlwaysAllow: false,
      capturedAt: interaction.capturedAt,
    },
    gemini: { sessionId: session.id },
  };
};

const kimiPending = (session: KimiSession): ConsolePending | null => {
  if (session.status === "idle" || session.status === "stopped") return null;
  const interaction = session.pendingInteractions
    .filter((item) => item.resolved !== true && (item.status === "observed" || item.status === "updated"))
    .sort((left, right) => left.capturedAt - right.capturedAt)[0];
  if (!interaction) return null;
  if (interaction.kind === "askUser") {
    return {
      kind: "question", requestId: interaction.observationId, readOnly: true,
      kimi: { sessionId: session.id },
      question: {
        id: interaction.observationId,
        questions: (interaction.questions.length ? interaction.questions : [{ question: interaction.detail }]).map((raw) => {
          const item = isRecord(raw) ? raw : {};
          return {
            header: typeof item.header === "string" ? item.header : interaction.title,
            question: typeof item.question === "string" ? item.question : interaction.detail,
            options: Array.isArray(item.options) ? item.options.flatMap((option) => isRecord(option) && typeof option.label === "string" ? [{ label: option.label, description: null }] : []) : [],
            multiSelect: item.multiSelect === true, allowOther: false, allowChat: false, readOnly: true,
          };
        }),
      },
    };
  }
  if (interaction.kind === "exitPlanMode") {
    return {
      kind: "plan", requestId: interaction.observationId, readOnly: true,
      kimi: { sessionId: session.id },
      plan: {
        id: interaction.observationId, toolName: interaction.title,
        plan: [interaction.plan ?? interaction.detail, interaction.planReadError].filter(Boolean).join("\n\n"),
        cwd: session.cwd, capturedAt: interaction.capturedAt,
      },
    };
  }
  return {
    kind: "permission",
    requestId: interaction.observationId,
    readOnly: true,
    permission: {
      id: interaction.observationId,
      toolName: interaction.toolName ?? interaction.title,
      summary: interaction.detail,
      cwd: session.cwd,
      canAlwaysAllow: false,
      capturedAt: interaction.capturedAt,
    },
    kimi: { sessionId: session.id },
  };
};

const sourceFromHookId = (id: string | undefined): ConsoleSource | undefined => {
  if (id === "claudeCode") return "claude";
  if (id === "codex") return "codex";
  if (id === "openCode") return "opencode";
  if (id === "mimo") return "mimo";
  if (id === "pi") return "pi";
  if (id === "deepSeekHarness") return "dsh";
  if (id === "zCode") return "zcode";
  if (id === "geminiCli") return "gemini";
  if (id === "kimiCode") return "kimi";
  return undefined;
};

const integrationName = (source: ConsoleSource): string =>
  source === "claude"
    ? "Claude Code"
    : source === "codex"
      ? "Codex"
      : source === "opencode"
        ? "OpenCode"
        : source === "mimo"
          ? "Mimo"
        : source === "pi"
          ? "PI"
          : source === "dsh"
            ? "DeepSeek Harness"
            : source === "zcode"
              ? "ZCode"
              : source === "gemini"
                ? "Gemini CLI"
                : "Kimi Code";

type SessionCounts = Record<ConsoleSource, number>;

const sessionCountFor = (
  source: ConsoleSource,
  counts: SessionCounts,
): number => counts[source];

const openCodeEntry = (session: OpenCodeSession): ConsoleEntry => ({
  key: `opencode:${session.pluginInstanceId}:${session.id}`,
  source: "opencode",
  sessionId: session.id,
  title: session.title,
  status: session.status,
  statusLabel: openCodeStatusLabel(session.status),
  cwd: session.cwd,
  updatedAt: session.updatedAt,
  activities: session.activities.map((activity) => ({
    id: activity.id,
    tool: activity.tool,
    summary: activity.summary,
    status: activity.status,
    updatedAt: activity.updatedAt,
  })),
  outputs: session.outputs.map((output) => ({ id: output.id, text: output.text })),
  pending: openCodePending(session),
});

const piEntry = (session: PiSession): ConsoleEntry => ({
  key: piSessionKey(session),
  source: "pi",
  sessionId: session.id,
  title: session.title,
  status: session.status,
  statusLabel: piStatusLabel(session.status),
  cwd: session.cwd,
  updatedAt: session.updatedAt,
  activities: session.activities.map((activity) => ({
    id: activity.id,
    tool: activity.tool,
    summary: activity.summary,
    status: activity.status,
    updatedAt: activity.updatedAt,
  })),
  outputs: session.outputs.map((output) => ({ id: output.id, text: output.text })),
  pending: piPending(session),
});

const mimoEntry = (session: MimoSession): ConsoleEntry => ({
  key: `mimo:${session.pluginInstanceId}:${session.id}`,
  source: "mimo",
  sessionId: session.id,
  title: session.title,
  status: session.status,
  statusLabel: mimoStatusLabel(session.status),
  cwd: session.cwd,
  updatedAt: session.updatedAt,
  activities: session.activities.map((activity) => ({ id: activity.id, tool: activity.tool, summary: activity.summary, status: activity.status, updatedAt: activity.updatedAt })),
  outputs: session.outputs.map((output) => ({ id: output.id, text: output.text })),
  pending: mimoPending(session),
});

const dshEntry = (session: DshSession): ConsoleEntry => ({
  key: dshSessionKey(session),
  source: "dsh",
  sessionId: session.id,
  title: session.title,
  status: session.status,
  statusLabel: dshStatusLabel(session.status),
  cwd: session.cwd,
  updatedAt: session.updatedAt,
  activities: session.activities.map((activity) => ({
    id: activity.id,
    tool: activity.tool,
    summary: activity.summary,
    status: activity.status,
    updatedAt: activity.updatedAt,
  })),
  outputs: session.outputs.map((output) => ({ id: output.id, text: output.text })),
  pending: dshPending(session),
});

const zcodeEntry = (session: ZCodeSession): ConsoleEntry => ({
  key: zcodeSessionKey(session),
  source: "zcode",
  sessionId: session.id,
  title: session.title,
  status: session.status,
  statusLabel: zcodeSessionStatusLabel(session),
  cwd: session.cwd,
  updatedAt: session.updatedAt,
  activities: session.activities.map((activity) => ({
    id: activity.id,
    tool: activity.tool,
    summary: activity.summary,
    status: activity.status,
    updatedAt: activity.updatedAt,
  })),
  outputs: session.outputs.map((output) => ({ id: output.id, text: output.text })),
  pending: zcodePending(session),
});

const geminiEntry = (session: GeminiSession): ConsoleEntry => ({
  key: geminiSessionKey(session),
  source: "gemini",
  sessionId: session.id,
  title: session.title,
  status: session.status,
  statusLabel: geminiStatusLabel(session.status),
  cwd: session.cwd,
  updatedAt: session.updatedAt,
  activities: session.activities.map((activity) => ({ id: activity.id, tool: activity.tool, summary: activity.summary, status: activity.status, updatedAt: activity.updatedAt })),
  outputs: session.outputs.map((output) => ({ id: output.id, text: output.text })),
  pending: geminiPending(session),
});

const kimiEntry = (session: KimiSession): ConsoleEntry => ({
  key: kimiSessionKey(session),
  source: "kimi",
  sessionId: session.id,
  title: session.title,
  status: session.status,
  statusLabel: kimiStatusLabel(session.status),
  cwd: session.cwd,
  updatedAt: session.updatedAt,
  activities: session.activities.map((activity) => ({ id: activity.id, tool: activity.tool, summary: activity.summary, status: activity.status, updatedAt: activity.updatedAt })),
  outputs: session.outputs.map((output) => ({ id: output.id, text: output.text })),
  pending: kimiPending(session),
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
  const opencode = isRecord(raw.opencode)
    ? { ...emptyOpenCode, ...(raw.opencode as OpenCodeSnapshot) }
    : emptyOpenCode;
  const mimo = isRecord(raw.mimo)
    ? { ...emptyMimo, ...(raw.mimo as MimoSnapshot) }
    : emptyMimo;
  const pi = isRecord(raw.pi)
    ? { ...emptyPi, ...(raw.pi as PiSnapshot) }
    : emptyPi;
  const dsh = isRecord(raw.dsh)
    ? { ...emptyDsh, ...(raw.dsh as DshSnapshot) }
    : emptyDsh;
  const zcode = isRecord(raw.zcode)
    ? { ...emptyZCode, ...(raw.zcode as ZCodeSnapshot) }
    : emptyZCode;
  const gemini = isRecord(raw.gemini)
    ? { ...emptyGemini, ...(raw.gemini as GeminiSnapshot) }
    : emptyGemini;
  const kimi = isRecord(raw.kimi)
    ? { ...emptyKimi, ...(raw.kimi as KimiSnapshot) }
    : emptyKimi;

  const entries = [
    ...claude.sessions.map(claudeEntry),
    ...codex.sessions.map((session) => codexEntry(session, codex.interactions)),
    ...opencode.sessions.map(openCodeEntry),
    ...mimo.sessions.map(mimoEntry),
    ...pi.sessions.map(piEntry),
    ...dsh.sessions.map(dshEntry),
    ...zcode.sessions.map(zcodeEntry),
    ...gemini.sessions.map(geminiEntry),
    ...kimi.sessions.map(kimiEntry),
  ].sort(compareEntries);

  const counts: SessionCounts = {
    claude: claude.sessions.length,
    codex: codex.sessions.length,
    opencode: opencode.sessions.length,
    mimo: mimo.sessions.length,
    pi: pi.sessions.length,
    dsh: dsh.sessions.length,
    zcode: zcode.sessions.length,
    gemini: gemini.sessions.length,
    kimi: kimi.sessions.length,
  };

  return {
    generatedAt: typeof raw.generatedAt === "number" ? raw.generatedAt : 0,
    allowApprovals: raw.allowApprovals === true,
    entries,
    integrations: buildIntegrations(raw, counts),
  };
};

const buildIntegrations = (
  raw: RawSnapshot,
  counts: SessionCounts,
): ConsoleIntegration[] => {
  const hookStatuses = Array.isArray(raw.integrations) ? raw.integrations : [];
  if (hookStatuses.length === 0) {
    // Older console/servers without install state fall back to the basic
    // connected summary. With no install report we cannot prove a hook is
    // missing, so treat every agent as installed and keep it visible.
    return [
      {
        source: "claude",
        name: integrationName("claude"),
        connected: claudeConnected(raw),
        error: integrationError(raw, "claude"),
        sessionCount: counts.claude,
        agentInstalled: true,
        hookInstalled: true,
      },
      {
        source: "codex",
        name: integrationName("codex"),
        connected: codexConnected(raw),
        error: integrationError(raw, "codex"),
        sessionCount: counts.codex,
        agentInstalled: true,
        hookInstalled: true,
      },
      {
        source: "opencode",
        name: integrationName("opencode"),
        connected: opencodeConnected(raw),
        error: integrationError(raw, "opencode"),
        sessionCount: counts.opencode,
        agentInstalled: true,
        hookInstalled: true,
      },
      {
        source: "mimo",
        name: integrationName("mimo"),
        connected: mimoConnected(raw),
        error: integrationError(raw, "mimo"),
        sessionCount: counts.mimo,
        agentInstalled: true,
        hookInstalled: true,
      },
      {
        source: "pi",
        name: integrationName("pi"),
        connected: piConnected(raw),
        error: integrationError(raw, "pi"),
        sessionCount: counts.pi,
        agentInstalled: true,
        hookInstalled: true,
      },
      {
        source: "dsh",
        name: integrationName("dsh"),
        connected: dshConnected(raw),
        error: integrationError(raw, "dsh"),
        sessionCount: counts.dsh,
        agentInstalled: true,
        hookInstalled: true,
      },
      {
        source: "zcode",
        name: integrationName("zcode"),
        connected: zcodeConnected(raw),
        error: integrationError(raw, "zcode"),
        sessionCount: counts.zcode,
        agentInstalled: true,
        hookInstalled: true,
      },
      {
        source: "gemini",
        name: integrationName("gemini"),
        connected: geminiConnected(raw),
        error: integrationError(raw, "gemini"),
        sessionCount: counts.gemini,
        agentInstalled: true,
        hookInstalled: true,
      },
      ...(isRecord(raw.kimi) ? [{
        source: "kimi" as const,
        name: integrationName("kimi"),
        connected: kimiConnected(raw),
        error: integrationError(raw, "kimi"),
        sessionCount: counts.kimi,
        agentInstalled: true,
        hookInstalled: true,
      }] : []),
    ];
  }

  return hookStatuses.flatMap((status) => {
    const source = sourceFromHookId(status.id);
    if (!source) return [];
    const connected =
      source === "claude"
        ? claudeConnected(raw)
        : source === "codex"
          ? codexConnected(raw)
          : source === "opencode"
            ? opencodeConnected(raw)
            : source === "mimo"
              ? mimoConnected(raw)
            : source === "pi"
              ? piConnected(raw)
              : source === "dsh"
                ? dshConnected(raw)
                : source === "zcode"
                  ? zcodeConnected(raw)
                  : source === "gemini"
                    ? geminiConnected(raw)
                    : kimiConnected(raw);
    return [
      {
        source,
        name: status.name ?? integrationName(source),
        connected,
        error: integrationError(raw, source),
        sessionCount: sessionCountFor(source, counts),
        agentInstalled: status.agentInstalled === true,
        hookInstalled: status.hookInstalled === true,
      },
    ];
  });
};

const claudeConnected = (raw: RawSnapshot): boolean =>
  isRecord(raw.claude) ? raw.claude.connected === true : false;
const codexConnected = (raw: RawSnapshot): boolean =>
  isRecord(raw.codex) ? raw.codex.connected === true : false;
const opencodeConnected = (raw: RawSnapshot): boolean =>
  isRecord(raw.opencode) ? raw.opencode.connected === true : false;
const mimoConnected = (raw: RawSnapshot): boolean =>
  isRecord(raw.mimo) ? raw.mimo.connected === true : false;
const piConnected = (raw: RawSnapshot): boolean =>
  isRecord(raw.pi) ? raw.pi.connected === true : false;
const dshConnected = (raw: RawSnapshot): boolean =>
  isRecord(raw.dsh) ? raw.dsh.connected === true : false;
const zcodeConnected = (raw: RawSnapshot): boolean =>
  isRecord(raw.zcode) ? raw.zcode.connected === true : false;
const geminiConnected = (raw: RawSnapshot): boolean =>
  isRecord(raw.gemini) ? raw.gemini.connected === true : false;
const kimiConnected = (raw: RawSnapshot): boolean =>
  isRecord(raw.kimi) ? raw.kimi.connected === true : false;

const integrationError = (raw: RawSnapshot, source: ConsoleSource): string | null => {
  const value =
    source === "claude"
      ? isRecord(raw.claude)
        ? raw.claude.integrationError
        : null
      : source === "codex"
        ? isRecord(raw.codex)
          ? raw.codex.integrationError
          : null
        : source === "opencode"
          ? isRecord(raw.opencode)
            ? raw.opencode.integrationError
            : null
          : source === "mimo"
            ? isRecord(raw.mimo)
              ? raw.mimo.integrationError
              : null
          : source === "pi"
            ? isRecord(raw.pi)
              ? raw.pi.integrationError
              : null
            : source === "dsh"
              ? isRecord(raw.dsh)
                ? raw.dsh.integrationError
                : null
              : source === "zcode"
                ? isRecord(raw.zcode)
                  ? raw.zcode.integrationError
                  : null
                : source === "gemini"
                  ? isRecord(raw.gemini)
                    ? raw.gemini.integrationError
                    : null
                  : isRecord(raw.kimi)
                    ? raw.kimi.integrationError
                    : null;
  return (value ?? null) as string | null;
};

/** Whether an agent card should be shown: only when its hook is installed,
 * mirroring the desktop's hook settings list. Agents the software reports as
 * not installed never appear in the console. */
export const integrationVisible = (integration: ConsoleIntegration): boolean =>
  integration.hookInstalled;

/** The two states the user wants on a card: hook missing, or session count. */
export const integrationStatusLabel = (integration: ConsoleIntegration): string =>
  integration.hookInstalled ? integration.sessionCount + " 个会话" : "hook未安装";

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
  PENDING_LABELS[pending.kind];

/**
 * Whether the console may submit this request: remote approvals must be on and
 * the request must not be one of the read-only Codex kinds.
 */
export const canAct = (
  pending: ConsolePending | null,
  allowApprovals: boolean,
): boolean => pending !== null && !pending.readOnly && allowApprovals;
