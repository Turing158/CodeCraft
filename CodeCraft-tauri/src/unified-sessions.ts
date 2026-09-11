import {
  sessionLiveContent,
  sessionLiveStatusText,
  sessionStatusLabel,
  type ClaudeLiveContent,
  type ClaudeSession,
  type ClaudeSessionStatus,
} from "./claude-sessions";
import {
  codexStatusLabel,
  type CodexSession,
  type CodexSessionStatus,
} from "./codex-sessions";
import {
  openCodeSessionKey,
  openCodeStatusLabel,
  type OpenCodeSession,
  type OpenCodeSessionStatus,
} from "./opencode-sessions";
import {
  piSessionKey,
  piStatusLabel,
  type PiSession,
  type PiSessionStatus,
} from "./pi-sessions";
import {
  dshSessionKey,
  dshStatusLabel,
  type DshSession,
  type DshSessionStatus,
} from "./dsh-sessions";
import {
  zcodeSessionKey,
  zcodeSessionStatusLabel,
  type ZCodeSession,
  type ZCodeSessionStatus,
} from "./zcode-sessions";
import {
  mimoSessionKey,
  mimoStatusLabel,
  type MimoSession,
  type MimoSessionStatus,
} from "./mimo-sessions";
import {
  geminiSessionKey,
  geminiStatusLabel,
  type GeminiSession,
  type GeminiSessionStatus,
} from "./gemini-sessions";
import {
  kimiSessionKey,
  kimiStatusLabel,
  type KimiSession,
  type KimiSessionStatus,
} from "./kimi-sessions";

export type UnifiedSession =
  | ClaudeSession
  | CodexSession
  | OpenCodeSession
  | PiSession
  | DshSession
  | ZCodeSession
  | MimoSession
  | GeminiSession
  | KimiSession;

const CODEX_VISUAL_STATUS: Record<CodexSessionStatus, ClaudeSessionStatus> = {
  working: "working",
  waitingForInput: "waiting",
  waitingForApproval: "attention",
  idle: "idle",
  stopped: "stopped",
};

const OPENCODE_VISUAL_STATUS: Record<
  OpenCodeSessionStatus,
  ClaudeSessionStatus
> = {
  working: "working",
  waitingForInput: "waiting",
  waitingForApproval: "attention",
  toolRunning: "working",
  toolFailed: "toolFailed",
  idle: "idle",
  stopped: "stopped",
};

const PI_VISUAL_STATUS: Record<PiSessionStatus, ClaudeSessionStatus> = {
  working: "working",
  waitingForInput: "waiting",
  waitingForApproval: "attention",
  toolRunning: "working",
  stopped: "stopped",
  idle: "idle",
};

const DSH_VISUAL_STATUS: Record<DshSessionStatus, ClaudeSessionStatus> = {
  working: "working",
  waitingForInput: "waiting",
  waitingForApproval: "attention",
  toolRunning: "working",
  toolFailed: "toolFailed",
  stopped: "stopped",
  idle: "idle",
};

const ZCODE_VISUAL_STATUS: Record<ZCodeSessionStatus, ClaudeSessionStatus> = {
  working: "working",
  waitingForInput: "waiting",
  waitingForApproval: "attention",
  toolRunning: "working",
  toolFailed: "toolFailed",
  stopped: "stopped",
  idle: "idle",
};
const MIMO_VISUAL_STATUS: Record<MimoSessionStatus, ClaudeSessionStatus> = {
  working: "working",
  waitingForInput: "waiting",
  waitingForApproval: "attention",
  toolRunning: "working",
  toolFailed: "toolFailed",
  stopped: "stopped",
  idle: "idle",
};
const GEMINI_VISUAL_STATUS: Record<GeminiSessionStatus, ClaudeSessionStatus> = {
  working: "working",
  waitingForInput: "waiting",
  toolRunning: "working",
  toolCompleted: "working",
  toolFailed: "toolFailed",
  idle: "idle",
  stopped: "stopped",
};
const KIMI_VISUAL_STATUS: Record<KimiSessionStatus, ClaudeSessionStatus> = {
  working: "working",
  waitingForInput: "waiting",
  toolRunning: "working",
  toolCompleted: "working",
  toolFailed: "toolFailed",
  idle: "idle",
  stopped: "stopped",
};

const LIVE_STATUS_RANK: Record<ClaudeSessionStatus, number> = {
  working: 0,
  toolFailed: 1,
  attention: 2,
  waiting: 3,
  stopped: 4,
  idle: 5,
};

export const isCodexSession = (
  session: UnifiedSession,
): session is CodexSession => "pendingInteractionId" in session;

export const isOpenCodeSession = (
  session: UnifiedSession,
): session is OpenCodeSession => "pendingReviews" in session && !("source" in session && session.source === "mimo");

export const isPiSession = (
  session: UnifiedSession,
): session is PiSession => "installId" in session;

export const isDshSession = (
  session: UnifiedSession,
): session is DshSession => "bridgeInstanceId" in session;

export const isZCodeSession = (
  session: UnifiedSession,
): session is ZCodeSession => "reviewState" in session;
export const isMimoSession = (session: UnifiedSession): session is MimoSession =>
  "pendingReviews" in session && "source" in session && session.source === "mimo";
export const isGeminiSession = (session: UnifiedSession): session is GeminiSession =>
  "pendingInteractions" in session && "integrationStatus" in session && !("kimiSessionId" in session);
export const isKimiSession = (session: UnifiedSession): session is KimiSession =>
  "kimiSessionId" in session;

export const unifiedSessionSource = (
  session: UnifiedSession,
): "claude" | "codex" | "opencode" | "pi" | "dsh" | "zcode" | "mimo" | "gemini" | "kimi" =>
  isCodexSession(session)
    ? "codex"
    : isOpenCodeSession(session)
      ? "opencode"
      : isPiSession(session)
        ? "pi"
        : isDshSession(session)
          ? "dsh"
          : isZCodeSession(session)
            ? "zcode"
            : isMimoSession(session)
              ? "mimo"
              : isGeminiSession(session)
                ? "gemini"
                : isKimiSession(session)
                  ? "kimi"
                  : "claude";

export const unifiedSessionKey = (session: UnifiedSession): string =>
  isOpenCodeSession(session)
    ? openCodeSessionKey(session)
    : isPiSession(session)
      ? piSessionKey(session)
      : isDshSession(session)
        ? dshSessionKey(session)
        : isZCodeSession(session)
          ? zcodeSessionKey(session)
          : isMimoSession(session)
            ? mimoSessionKey(session)
            : isGeminiSession(session)
              ? geminiSessionKey(session)
              : isKimiSession(session)
                ? kimiSessionKey(session)
                : session.id;

export const unifiedSessionStatusLabel = (session: UnifiedSession): string =>
  isCodexSession(session)
    ? codexStatusLabel(session.status)
    : isOpenCodeSession(session)
      ? openCodeStatusLabel(session.status)
      : isPiSession(session)
        ? piStatusLabel(session.status)
        : isDshSession(session)
          ? dshStatusLabel(session.status)
          : isZCodeSession(session)
            ? zcodeSessionStatusLabel(session)
            : isMimoSession(session)
              ? mimoStatusLabel(session.status)
              : isGeminiSession(session)
                ? geminiStatusLabel(session.status)
                : isKimiSession(session)
                  ? kimiStatusLabel(session.status)
                  : sessionStatusLabel(session.status);

export const unifiedSessionVisualStatus = (
  session: UnifiedSession,
): ClaudeSessionStatus =>
  isCodexSession(session)
    ? CODEX_VISUAL_STATUS[session.status]
    : isOpenCodeSession(session)
      ? OPENCODE_VISUAL_STATUS[session.status]
      : isPiSession(session)
        ? PI_VISUAL_STATUS[session.status]
        : isDshSession(session)
          ? DSH_VISUAL_STATUS[session.status]
          : isZCodeSession(session)
            ? ZCODE_VISUAL_STATUS[session.status]
            : isMimoSession(session)
              ? MIMO_VISUAL_STATUS[session.status]
              : isGeminiSession(session)
                ? GEMINI_VISUAL_STATUS[session.status]
                : isKimiSession(session)
                  ? KIMI_VISUAL_STATUS[session.status]
                  : session.status;

export const unifiedSessionIsRunning = (session: UnifiedSession): boolean => {
  const status = unifiedSessionVisualStatus(session);
  return status !== "idle" && status !== "stopped";
};

export const unifiedSessionLiveContent = (
  session: UnifiedSession,
): ClaudeLiveContent => {
  if (
    !isCodexSession(session) &&
    !isOpenCodeSession(session) &&
    !isPiSession(session) &&
    !isDshSession(session) &&
    !isZCodeSession(session) &&
    !isMimoSession(session)
    && !isGeminiSession(session)
    && !isKimiSession(session)
  ) {
    return sessionLiveContent(session);
  }

  const activity =
    [...session.activities]
      .reverse()
      .find((item) => item.status === "running") ??
    session.activities[session.activities.length - 1];
  const output = session.outputs[session.outputs.length - 1];
  return activity
    ? { kind: "tool", text: `${activity.tool} · ${activity.summary}` }
    : output
      ? { kind: "output", text: output.text }
      : {
          kind: "status",
          text: isCodexSession(session)
            ? codexStatusLabel(session.status)
            : isOpenCodeSession(session)
              ? openCodeStatusLabel(session.status)
              : isPiSession(session)
                ? piStatusLabel(session.status)
                : isDshSession(session)
                ? dshStatusLabel(session.status)
                  : isZCodeSession(session)
                    ? zcodeSessionStatusLabel(session)
                    : isGeminiSession(session)
                      ? geminiStatusLabel(session.status)
                      : isKimiSession(session)
                        ? kimiStatusLabel(session.status)
                        : mimoStatusLabel(session.status),
        };
};

export const unifiedSessionLiveStatusText = (
  session: UnifiedSession,
): string => {
  if (
    !isCodexSession(session) &&
    !isOpenCodeSession(session) &&
    !isPiSession(session) &&
    !isDshSession(session) &&
    !isZCodeSession(session) &&
    !isMimoSession(session)
    && !isGeminiSession(session)
    && !isKimiSession(session)
  ) {
    return sessionLiveStatusText(session);
  }
  if (
    (session.status === "working" || session.status === "toolRunning") &&
    session.activities.some((activity) => activity.status === "running")
  ) {
    return "调用工具中";
  }
  return isCodexSession(session)
    ? codexStatusLabel(session.status)
    : isOpenCodeSession(session)
      ? openCodeStatusLabel(session.status)
      : isPiSession(session)
        ? piStatusLabel(session.status)
        : isDshSession(session)
          ? dshStatusLabel(session.status)
          : isZCodeSession(session)
            ? zcodeSessionStatusLabel(session)
            : isGeminiSession(session)
              ? geminiStatusLabel(session.status)
              : isKimiSession(session)
                ? kimiStatusLabel(session.status)
                : mimoStatusLabel(session.status);
};

export const hasUnifiedWorkingSession = (
  sessions: Iterable<UnifiedSession>,
): boolean => {
  for (const session of sessions) {
    if (unifiedSessionVisualStatus(session) === "working") return true;
  }
  return false;
};

export const primaryUnifiedLiveSession = (
  sessions: Iterable<UnifiedSession>,
): UnifiedSession | undefined => {
  let primary: UnifiedSession | undefined;
  for (const session of sessions) {
    const status = unifiedSessionVisualStatus(session);
    if (status === "idle") continue;
    if (!primary) {
      primary = session;
      continue;
    }

    const rankDifference =
      LIVE_STATUS_RANK[status] -
      LIVE_STATUS_RANK[unifiedSessionVisualStatus(primary)];
    if (rankDifference < 0 || (rankDifference === 0 && session.updatedAt > primary.updatedAt)) {
      primary = session;
    }
  }
  return primary;
};
