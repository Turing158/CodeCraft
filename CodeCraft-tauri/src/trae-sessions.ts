import type { ClaudeSession, ClaudeSessionStatus, ClaudeQuestionRequest, ClaudePlanRequest, ClaudePermissionRequest } from "./claude-sessions";
import type { Action, Answer, QuestionInput, Target } from "./trae-contract.generated";
import { currentLocale, translateText } from "./i18n";

export type { Action, Answer, QuestionInput, Target } from "./trae-contract.generated";
export interface TraeRequest {
  target: Target;
  kind: "permission" | "question" | "plan";
  channel: "hook" | "mcp";
  toolUseId: string;
  toolName: string;
  arguments: Record<string, unknown>;
  state: string;
  expiresAt: string;
  plan: { planId: string; revision: number; contentHash: string; markdown: string; documentPath: string } | null;
  error: { error: { code: string; message: string } } | null;
}
export interface TraeNativeSession {
  source: "trae";
  sessionKey: string;
  sessionId: string;
  installationId: string;
  traeInstanceId: string;
  cwd: string;
  workspaceRoots: string[];
  title: string;
  status: string;
  turnEpoch: number;
  taskId: string | null;
  nativeNotice: { type: string; message: string; readOnly: true } | null;
  nativeInteractions?: TraeNativeInteraction[];
  output: string;
  messages?: { id: string; role: "user" | "assistant"; text: string; at: string }[];
  historyTruncated?: boolean;
  startedAt?: string;
  updatedAt: string;
  activities: { id?: string; tool: string; toolUseId: string; status: string; arguments?: unknown; result?: unknown; at: string; startedAt?: string }[];
}
export interface TraeNativeInteraction {
  id: string; kind: "question" | "plan"; toolUseId: string | null;
  arguments: Record<string, unknown>; message: string; readOnly: true;
  capturedAt: string; documentPath: string | null; plan: string | null;
  planSource?: "notification" | "session_file" | "ambiguous";
  documentToolUseId?: string;
}
export interface TraeReview {
  key: string; sessionKey: string; kind: "permission" | "question" | "plan";
  request?: TraeRequest; native?: TraeNativeInteraction;
}
export function traeReviews(snapshot: TraeSnapshot): TraeReview[] {
  if (!snapshot.connected) return [];
  const sessions = new Set(snapshot.sessions.map(s => s.sessionKey));
  return [
    ...snapshot.requests.filter(r => r.kind === "permission" && r.channel === "hook" && traePending(r) && r.target.appEpoch === snapshot.appEpoch && sessions.has(r.target.sessionKey))
      .map(r => ({ key: `${snapshot.appEpoch}:permission:${r.target.requestId}`, sessionKey: r.target.sessionKey, kind: "permission" as const, request: r })),
    ...snapshot.sessions.flatMap(s => (s.nativeInteractions ?? []).filter(n => n.kind === "question" || n.kind === "plan")
      .map(n => ({ key: `${snapshot.appEpoch}:native:${n.id}`, sessionKey: s.sessionKey, kind: n.kind, native: n }))),
  ];
}
export class TraeReviewQueue {
  private revealed = new Set<string>();
  next(reviews: TraeReview[], visibleKey?: string): TraeReview | undefined {
    const keys = new Set(reviews.map(r => r.key));
    for (const key of this.revealed) if (!keys.has(key)) this.revealed.delete(key);
    return reviews.find(r => r.key === visibleKey) ?? reviews.find(r => !this.revealed.has(r.key));
  }
  mark(key: string) { this.revealed.add(key); }
}
export function traeQuestionRequest(review: TraeReview): ClaudeQuestionRequest {
  const native = review.native!;
  const raw = native.arguments.questions;
  const questions = Array.isArray(raw) ? raw.slice(0, 8).filter(q => q && typeof q === "object") as Record<string, unknown>[] : [];
  return { id: review.key, questions: (questions.length ? questions : [{ question: native.message || "Trae 正在等待回答，通知未提供完整题目，请前往 Trae 查看。" }]).map(q => ({
    header: typeof q.header === "string" ? q.header : "Trae 问题",
    question: typeof q.question === "string" ? q.question : typeof q.prompt === "string" ? q.prompt : native.message || "请前往 Trae 查看完整题目。",
    options: (Array.isArray(q.options) ? q.options.slice(0, 12) : []).flatMap((o: unknown) => {
      if (typeof o === "string") return [{ label: o, description: null }];
      if (o && typeof o === "object" && "label" in o && typeof o.label === "string") return [{ label: o.label, description: "description" in o && typeof o.description === "string" ? o.description : null }];
      return [];
    }),
    multiSelect: q.multiSelect === true || q.kind === "multi", allowOther: false, allowChat: false, readOnly: true,
    answerMode: "仅供查看，请前往 Trae 中回答",
  })) };
}
export function traePlanRequest(review: TraeReview): ClaudePlanRequest {
  const native = review.native;
  let plan = native?.plan;
  if (plan) {
    const path = native?.documentPath?.replace(/^\\\\\?\\(?=[a-z]:[\\/])/i, "");
    const fence = "`".repeat(Math.max(2, ...(path?.match(/`+/g) ?? []).map(s => s.length)) + 1);
    plan = [
      native?.planSource === "session_file" ? "来源：当前会话本轮生成的计划文档。" : "",
      path ? `文档路径：\n\n${fence}\n${path}\n${fence}` : "",
      plan,
    ].filter(Boolean).join("\n\n");
  } else if (native?.planSource === "ambiguous") {
    plan = "当前会话本轮写入了多个计划文档，无法确定本次审阅对应哪一份。请前往 Trae 查看。";
  } else {
    const message = native?.message;
    plan = message && !/^Tool 'NotifyUser' requires user confirmation\s*$/.test(message)
      ? message : "Trae 正在等待计划确认，通知未提供可可靠关联的计划正文，请前往 Trae 查看完整计划。";
  }
  return { id: review.key, toolName: "Trae Plan / Spec", plan, cwd: null, capturedAt: Date.parse(native?.capturedAt ?? "") || 0 };
}
export function traePermissionRequest(review: TraeReview, snapshot: TraeSnapshot): ClaudePermissionRequest {
  return { id: review.key, toolName: review.request!.toolName, summary: formatTraeToolArguments(review.request!.arguments), cwd: snapshot.sessions.find(s => s.sessionKey === review.sessionKey)?.cwd ?? null, canAlwaysAllow: false, capturedAt: 0 };
}
/** Display decoded string values without changing the original approval payload. */
export function formatTraeToolArguments(args: Record<string, unknown>): string {
  const entries = Object.entries(args);
  if (!entries.length) return "{}";
  const priority = (key: string) => ["file_path", "path", "command"].includes(key) ? 0 : 1;
  return entries.sort(([a], [b]) => priority(a) - priority(b))
    .map(([key, value]) => `${key}:\n${typeof value === "string" ? value : JSON.stringify(value, null, 2)}`)
    .join("\n\n");
}
export interface TraeTask {
  taskId: string; planId: string; version: number; title: string;
  primaryRoot: string; workspaceRoots: string[]; documentPath: string;
  state: string; sessionKey: string | null; turnEpoch: number;
  revision: number; contentHash: string; approved: boolean; expiresAt: string;
}
export interface TraeGrant {
  grantId: string; version: number; sessionKey: string; taskId: string | null;
  toolName: string; toolUseId: string; state: string; resolution: string | null;
}
export interface TraeSnapshot {
  source: "trae"; appEpoch: string; version: number; connected: boolean;
  integrationError?: unknown;
  capabilities: { toolApproval: boolean; mcpQuestions: boolean; mcpPlanReview: boolean; nativeObservation: boolean; reason: string; verifiedVersion: string | null };
  sessions: TraeNativeSession[]; requests: TraeRequest[]; tasks: TraeTask[]; grants: TraeGrant[];
}
export const emptyTraeSnapshot = (): TraeSnapshot => ({
  source: "trae", appEpoch: "", version: 0, connected: false,
  capabilities: { toolApproval: false, mcpQuestions: false, mcpPlanReview: false, nativeObservation: false, reason: "等待 Trae 实机验证", verifiedVersion: null },
  sessions: [], requests: [], tasks: [], grants: [],
});
export function readTraeSnapshot(value: unknown): TraeSnapshot {
  if (!value || typeof value !== "object" || (value as TraeSnapshot).source !== "trae") return emptyTraeSnapshot();
  const v = value as TraeSnapshot;
  return { ...emptyTraeSnapshot(), ...v, sessions: Array.isArray(v.sessions) ? v.sessions : [], requests: Array.isArray(v.requests) ? v.requests : [], tasks: Array.isArray(v.tasks) ? v.tasks : [], grants: Array.isArray(v.grants) ? v.grants : [] };
}
export const traePending = (request: TraeRequest) => ["pending", "user_decided", "delivery_prepared"].includes(request.state);
export const traeStatus = (status: string): ClaudeSessionStatus => ({ waitingForInput: "waiting", waitingForApproval: "attention", working: "working", stopped: "stopped", idle: "idle" } as Record<string, ClaudeSessionStatus>)[status] ?? "attention";
export interface TraeUnifiedSession extends ClaudeSession { source: "trae"; sessionKey: string; cwd: string; detailNotice: string; }
export function toTraeUnifiedSession(s: TraeNativeSession): TraeUnifiedSession {
  const at = Date.parse(s.updatedAt) || 0;
  const t = (text: string) => translateText(text, currentLocale());
  const display = (value: unknown) => typeof value === "string" ? value : JSON.stringify(value, null, 2);
  return { source: "trae", sessionKey: s.sessionKey, cwd: s.cwd.replace(/^\\\\\?\\(?=[a-z]:[\\/])/i, ""), id: s.sessionKey, title: s.title, status: traeStatus(s.status), startedAt: Date.parse(s.startedAt ?? "") || at, updatedAt: at, question: null, permission: null, plan: null,
    detailNotice: t(s.historyTruncated ? "仅显示近期采集记录，较早或过长的内容已截断。" : "显示已采集的提问、最终回复和工具活动；不包含接入前的完整历史或逐字输出。"),
    outputs: s.messages?.length ? s.messages.map(m => ({ id: m.id, text: m.text, role: m.role, capturedAt: Date.parse(m.at) || at })) : s.output ? [{ id: s.sessionKey, text: s.output, role: "assistant", capturedAt: at }] : [],
    activities: s.activities.map((a, i) => ({
      id: a.id || a.toolUseId || String(i), tool: a.tool,
      summary: [a.arguments !== undefined ? `${t("输入参数")}：\n${display(a.arguments)}` : "",
        a.result !== undefined ? `${t("执行结果")}：\n${display(a.result)}` : t(a.status === "running" ? "等待工具返回结果" : "未收到工具结果，无法确认执行是否成功")].filter(Boolean).join("\n\n"),
      status: a.status === "running" ? "running" : a.status === "failed" ? "failed" : a.status === "completed" ? "completed" : "unknown",
      startedAt: Date.parse(a.startedAt ?? a.at) || at, updatedAt: Date.parse(a.at) || at,
    })),
  };
}
export function validateTraeAnswers(input: QuestionInput, answers: Answer[]): string | null {
  if (answers.length !== input.questions.length) return "请回答每个问题";
  const seen = new Set<string>();
  for (const a of answers) {
    const q = input.questions.find(q => q.questionId === a.questionId);
    if (!q || seen.has(a.questionId)) return "问题已变化，请刷新";
    seen.add(a.questionId);
    if (a.status === "skipped") {
      if (q.required || a.selectedOptionIds.length || a.text !== null) return "必填问题不能跳过";
      continue;
    }
    const ids = new Set(a.selectedOptionIds);
    if (ids.size !== a.selectedOptionIds.length || [...ids].some(id => !q.options.some(o => o.optionId === id))) return "选项已变化，请刷新";
    if (ids.size < q.minSelections || ids.size > q.maxSelections) return "请选择要求数量的选项";
    if ((!q.allowText && a.text !== null) || (a.text !== null && [...a.text].length > 4096)) return "回答文本不符合要求";
    if (!ids.size && !a.text?.trim()) return "请填写回答或明确跳过";
  }
  return null;
}
export const requestIdentity = (r: TraeRequest) => JSON.stringify(r.target);
export function canDecide(snapshot: TraeSnapshot, r: TraeRequest): boolean {
  if (r.kind !== "permission" || r.channel !== "hook") return false;
  const current = snapshot.requests.find(x => requestIdentity(x) === requestIdentity(r));
  const capable = r.kind === "permission" ? snapshot.capabilities.toolApproval : r.kind === "question" ? snapshot.capabilities.mcpQuestions : snapshot.capabilities.mcpPlanReview;
  return snapshot.connected && capable && snapshot.appEpoch === r.target.appEpoch && current?.state === "pending" && Date.parse(r.expiresAt) > Date.now();
}
/** A failed network submission keeps its ID; editing the action starts a new decision. */
export class TraeDecisionIds {
  private ids = new Map<string, string>();
  for(r: TraeRequest, action: Action): string {
    const key = requestIdentity(r) + JSON.stringify(action);
    let id = this.ids.get(key);
    if (!id) { id = crypto.randomUUID(); this.ids.set(key, id); }
    return id;
  }
  clear() { this.ids.clear(); }
}
