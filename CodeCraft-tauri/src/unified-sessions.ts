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

export type UnifiedSession = ClaudeSession | CodexSession | OpenCodeSession;

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
): session is OpenCodeSession => "pluginInstanceId" in session;

export const unifiedSessionSource = (
  session: UnifiedSession,
): "claude" | "codex" | "opencode" =>
  isCodexSession(session)
    ? "codex"
    : isOpenCodeSession(session)
      ? "opencode"
      : "claude";

export const unifiedSessionKey = (session: UnifiedSession): string =>
  isOpenCodeSession(session) ? openCodeSessionKey(session) : session.id;

export const unifiedSessionStatusLabel = (session: UnifiedSession): string =>
  isCodexSession(session)
    ? codexStatusLabel(session.status)
    : isOpenCodeSession(session)
      ? openCodeStatusLabel(session.status)
      : sessionStatusLabel(session.status);

export const unifiedSessionVisualStatus = (
  session: UnifiedSession,
): ClaudeSessionStatus =>
  isCodexSession(session)
    ? CODEX_VISUAL_STATUS[session.status]
    : isOpenCodeSession(session)
      ? OPENCODE_VISUAL_STATUS[session.status]
      : session.status;

export const unifiedSessionLiveContent = (
  session: UnifiedSession,
): ClaudeLiveContent => {
  if (!isCodexSession(session) && !isOpenCodeSession(session)) {
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
            : openCodeStatusLabel(session.status),
        };
};

export const unifiedSessionLiveStatusText = (
  session: UnifiedSession,
): string => {
  if (!isCodexSession(session) && !isOpenCodeSession(session)) {
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
    : openCodeStatusLabel(session.status);
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
