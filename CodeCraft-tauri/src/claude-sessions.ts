export type ClaudeSessionStatus =
  | "working"
  | "waiting"
  | "attention"
  | "toolFailed"
  | "stopped"
  | "idle";

export interface ClaudeQuestionOption {
  label: string;
  description: string | null;
}

export interface ClaudeQuestion {
  header: string | null;
  question: string;
  options: ClaudeQuestionOption[];
  multiSelect: boolean;
  allowOther?: boolean;
  allowChat?: boolean;
  readOnly?: boolean;
  answerMode?: string;
}

export interface ClaudeQuestionRequest {
  id: string;
  questions: ClaudeQuestion[];
}

export interface ClaudePermissionRequest {
  id: string;
  toolName: string;
  summary: string;
  cwd: string | null;
  canAlwaysAllow: boolean;
  capturedAt: number;
}

export interface ClaudePlanRequest {
  id: string;
  toolName: string;
  plan: string;
  cwd: string | null;
  capturedAt: number;
}

export type ClaudeActivityStatus = "running" | "completed" | "failed";

export interface ClaudeActivity {
  id: string;
  tool: string;
  summary: string;
  status: ClaudeActivityStatus;
  startedAt: number;
  updatedAt: number;
}

export interface ClaudeOutputEntry {
  id: string;
  text: string;
}

export type ClaudeLiveContentKind = "tool" | "output" | "status";

export interface ClaudeLiveContent {
  kind: ClaudeLiveContentKind;
  text: string;
}

export interface ClaudeSession {
  id: string;
  status: ClaudeSessionStatus;
  title: string;
  startedAt: number;
  updatedAt: number;
  question: ClaudeQuestionRequest | null;
  permission: ClaudePermissionRequest | null;
  plan: ClaudePlanRequest | null;
  activities: ClaudeActivity[];
  outputs: ClaudeOutputEntry[];
}

export type ClaudeAnswerOptionKind = "provided" | "other" | "chat";

export interface ClaudeAnswerOption extends ClaudeQuestionOption {
  id: string;
  kind: ClaudeAnswerOptionKind;
}

export interface ClaudeSessionSnapshot {
  connected: boolean;
  integrationError: string | null;
  sessions: ClaudeSession[];
}

export type ClaudeSessionEntryView =
  | "detail"
  | "question"
  | "permission"
  | "plan";

const STATUS_LABELS: Record<ClaudeSessionStatus, string> = {
  working: "工作中",
  waiting: "等待输入",
  attention: "需要处理",
  toolFailed: "工具调用失败",
  stopped: "停止",
  idle: "空闲",
};

const sessionTimeFormatter = new Intl.DateTimeFormat("zh-CN", {
  hour: "2-digit",
  minute: "2-digit",
  hour12: false,
});

export function sessionStatusLabel(status: ClaudeSessionStatus): string {
  return STATUS_LABELS[status];
}

export function sessionEntryView(
  session: ClaudeSession,
): ClaudeSessionEntryView {
  if (session.plan) return "plan";
  if (session.permission) return "permission";
  if (session.question) return "question";
  return "detail";
}

export function formatSessionTime(timestamp: number): string {
  return sessionTimeFormatter.format(new Date(timestamp));
}

export function hasWorkingSession(sessions: ClaudeSession[]): boolean {
  return sessions.some((session) => session.status === "working");
}

export function hasActiveSession(sessions: ClaudeSession[]): boolean {
  return sessions.some((session) => session.status !== "idle");
}

/** Returns the highest-priority session worth surfacing in the collapsed live card. */
export function primaryLiveSession(
  sessions: ClaudeSession[],
): ClaudeSession | undefined {
  return sessions.find((session) => session.status !== "idle");
}

/** Selects the most useful single-line activity for a compact live session card. */
export function sessionLiveContent(session: ClaudeSession): ClaudeLiveContent {
  const runningActivity = [...session.activities]
    .reverse()
    .find((activity) => activity.status === "running");
  if (runningActivity) {
    return {
      kind: "tool",
      text: `${runningActivity.tool} · ${runningActivity.summary}`,
    };
  }

  const latestOutput = session.outputs[session.outputs.length - 1];
  if (latestOutput) {
    return { kind: "output", text: latestOutput.text };
  }

  const latestActivity = session.activities[session.activities.length - 1];
  if (latestActivity) {
    return {
      kind: "tool",
      text: `${latestActivity.tool} · ${latestActivity.summary}`,
    };
  }

  return { kind: "status", text: sessionStatusLabel(session.status) };
}

export function sessionLiveStatusText(session: ClaudeSession): string {
  if (
    session.status === "working" &&
    session.activities.some((activity) => activity.status === "running")
  ) {
    return "调用工具中";
  }
  return sessionStatusLabel(session.status);
}

export function questionAnswerOptions(
  requestId: string,
  question: ClaudeQuestion,
  questionIndex: number,
): ClaudeAnswerOption[] {
  const questionId = `${requestId}:question:${questionIndex}`;

  const options: ClaudeAnswerOption[] = question.options.map((option, index) => ({
      ...option,
      id: `${questionId}:option:${index}`,
      kind: "provided" as const,
    }));
  if (question.allowOther !== false) {
    options.push({
      id: `${questionId}:other`,
      kind: "other",
      label: "其他",
      description: "输入你自己的回答",
    });
  }
  if (question.allowChat !== false) {
    options.push({
      id: `${questionId}:chat`,
      kind: "chat",
      label: "再聊一下",
      description: "先和 Claude 讨论这个问题",
    });
  }
  return options;
}
