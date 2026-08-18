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

export type UnifiedSession = ClaudeSession | CodexSession;

const CODEX_VISUAL_STATUS: Record<CodexSessionStatus, ClaudeSessionStatus> = {
  working: "working",
  waitingForInput: "waiting",
  waitingForApproval: "attention",
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

export const unifiedSessionStatusLabel = (session: UnifiedSession): string =>
  isCodexSession(session)
    ? codexStatusLabel(session.status)
    : sessionStatusLabel(session.status);

export const unifiedSessionVisualStatus = (
  session: UnifiedSession,
): ClaudeSessionStatus =>
  isCodexSession(session) ? CODEX_VISUAL_STATUS[session.status] : session.status;

export const unifiedSessionLiveContent = (
  session: UnifiedSession,
): ClaudeLiveContent => {
  if (!isCodexSession(session)) return sessionLiveContent(session);

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
      : { kind: "status", text: codexStatusLabel(session.status) };
};

export const unifiedSessionLiveStatusText = (
  session: UnifiedSession,
): string => {
  if (!isCodexSession(session)) return sessionLiveStatusText(session);
  if (
    session.status === "working" &&
    session.activities.some((activity) => activity.status === "running")
  ) {
    return "调用工具中";
  }
  return codexStatusLabel(session.status);
};

export const hasUnifiedWorkingSession = (
  sessions: UnifiedSession[],
): boolean => sessions.some((session) => session.status === "working");

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
