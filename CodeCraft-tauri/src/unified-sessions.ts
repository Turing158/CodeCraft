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

export type UnifiedSession =
  | ClaudeSession
  | CodexSession
  | OpenCodeSession
  | PiSession
  | DshSession
  | ZCodeSession;

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
): session is OpenCodeSession => "pendingReviews" in session;

export const isPiSession = (
  session: UnifiedSession,
): session is PiSession => "installId" in session;

export const isDshSession = (
  session: UnifiedSession,
): session is DshSession => "bridgeInstanceId" in session;

export const isZCodeSession = (
  session: UnifiedSession,
): session is ZCodeSession => "reviewState" in session;

export const unifiedSessionSource = (
  session: UnifiedSession,
): "claude" | "codex" | "opencode" | "pi" | "dsh" | "zcode" =>
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
    !isZCodeSession(session)
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
                  : zcodeSessionStatusLabel(session),
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
    !isZCodeSession(session)
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
            : zcodeSessionStatusLabel(session);
};

export const hasUnifiedWorkingSession = (
  sessions: UnifiedSession[],
): boolean =>
  sessions.some(
    (session) => unifiedSessionVisualStatus(session) === "working",
  );

export const primaryUnifiedLiveSession = (
  sessions: UnifiedSession[],
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
