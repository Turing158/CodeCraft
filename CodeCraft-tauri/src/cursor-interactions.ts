export interface CursorQuestion { questionId: string; prompt: string; options: { optionId: string; label: string }[]; multiSelect: boolean; allowFreeText: boolean; required: boolean; allowSkip: boolean; maxTextLength: number }
export interface CursorAnswer { questionId: string; selectedOptionIds: string[]; text: string | null; skipped: boolean }
export type CursorDecision = { kind: "permission"; action: "allowOnce" | "deny" } | { kind: "answered"; answers: CursorAnswer[] } | { kind: "questionCancelled" } | { kind: "plan"; action: "approved" | "revise" | "rejected" | "cancelled"; feedback: string };
export interface CursorRequest {
  requestId: string; sessionKey: string; workflowId: string | null; origin: "hook" | "independentMcp"; title: string;
  createdAt: number; updatedAt: number; expiresAt: number; actionable: boolean; phase: string; transport: string; reason: string | null;
  submission: { requestId: string; appEpoch: string; requestVersion: number; argumentsDigest: string; plan: { planId: string; revision: number; contentDigest: string } | null };
  body: { kind: "permission"; content: { toolName: string; eventName: string; toolInput: unknown } }
    | { kind: "question"; content: { questions: CursorQuestion[] } }
    | { kind: "plan"; content: { title: string; overview: string; markdown: string; todos: { id: string; description: string; status: string; phaseId: string | null }[]; phases: { id: string; title: string; description: string }[] } };
  decision: { value: CursorDecision } | null;
}
export const cursorRecord = (v: unknown): v is Record<string, unknown> => !!v && typeof v === "object" && !Array.isArray(v);
const str = (v: unknown, max: number): v is string => typeof v === "string" && v.length <= max;
const id = (v: unknown): v is string => str(v, 128) && /^[A-Za-z0-9_.:-]+$/.test(v);
const number = (v: unknown): v is number => typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
const digest = (v: unknown): v is string => str(v, 64) && /^[a-f0-9]{64}$/.test(v);
const question = (v: unknown): v is CursorQuestion => cursorRecord(v) && id(v.questionId) && str(v.prompt,8192)
  && Array.isArray(v.options) && v.options.length <= 64 && v.options.every(o => cursorRecord(o) && id(o.optionId) && str(o.label,2048))
  && new Set(v.options.map(o => o.optionId)).size === v.options.length
  && [v.multiSelect,v.allowFreeText,v.required,v.allowSkip].every(x => typeof x === "boolean") && number(v.maxTextLength) && v.maxTextLength <= 4096;
const answer = (v: unknown): v is CursorAnswer => cursorRecord(v) && id(v.questionId)
  && Array.isArray(v.selectedOptionIds) && v.selectedOptionIds.length <= 64 && v.selectedOptionIds.every(id)
  && (v.text === null || str(v.text,8192)) && typeof v.skipped === "boolean";
function validDecision(value: unknown, body: Record<string,unknown>): boolean {
  if (value === null) return true;
  if (!cursorRecord(value) || !cursorRecord(value.value)) return false;
  const d=value.value;
  if (body.kind === "permission") return d.kind === "permission" && ["allowOnce","deny"].includes(String(d.action));
  if (body.kind === "plan") return d.kind === "plan" && ["approved","revise","rejected","cancelled"].includes(String(d.action)) && str(d.feedback,16384);
  if (d.kind === "questionCancelled") return true;
  return d.kind === "answered" && Array.isArray(d.answers) && d.answers.length <= 32 && d.answers.every(answer)
    && validateCursorAnswers((body.content as {questions:CursorQuestion[]}).questions,d.answers) === null;
}
export function readCursorRequests(value: unknown, appEpoch: string): CursorRequest[] {
  if (!Array.isArray(value) || value.length > 1024) return [];
  const seen = new Set<string>();
  return value.filter((v): v is CursorRequest => {
    if (!cursorRecord(v) || !id(v.requestId) || seen.has(v.requestId) || !digest(v.sessionKey) || !str(v.title,4096)
      || !["hook","independentMcp"].includes(String(v.origin)) || !(v.workflowId === null || id(v.workflowId))
      || !number(v.createdAt) || !number(v.updatedAt) || !number(v.expiresAt) || typeof v.actionable !== "boolean"
      || !["pending","decided","cancelled","expired","superseded","disconnected"].includes(String(v.phase))
      || !["notStarted","claimed","written","deliveryFailed","outcomeUnknown"].includes(String(v.transport))
      || !(v.reason === null || str(v.reason,256)) || !cursorRecord(v.submission) || !cursorRecord(v.body)) return false;
    const s = v.submission;
    if (s.requestId !== v.requestId || !str(s.appEpoch,128) || !number(s.requestVersion) || !digest(s.argumentsDigest)) return false;
    if (v.actionable && (s.appEpoch !== appEpoch || v.phase !== "pending")) return false;
    const c = v.body.content;
    if (!cursorRecord(c)) return false;
    if (v.body.kind === "question") {
      if (v.origin !== "independentMcp" || !Array.isArray(c.questions) || !c.questions.length || c.questions.length > 32 || !c.questions.every(question)
        || new Set(c.questions.map(q => q.questionId)).size !== c.questions.length || s.plan !== null) return false;
    } else if (v.body.kind === "plan") {
      if (v.origin !== "independentMcp" || !str(c.title,2048) || !str(c.overview,16384) || !str(c.markdown,131072)
        || !Array.isArray(c.todos) || c.todos.length > 256 || !c.todos.every(t => cursorRecord(t) && id(t.id) && str(t.description,8192) && (t.phaseId === null || id(t.phaseId)) && ["pending","inProgress","completed","cancelled"].includes(String(t.status)))
        || !Array.isArray(c.phases) || c.phases.length > 64 || !c.phases.every(p => cursorRecord(p) && id(p.id) && str(p.title,2048) && str(p.description,8192))
        || !cursorRecord(s.plan) || !id(s.plan.planId) || !number(s.plan.revision) || !digest(s.plan.contentDigest)) return false;
    } else if (v.body.kind === "permission") {
      if (v.origin !== "hook" || !str(c.toolName,2048) || !["preToolUse","beforeMCPExecution"].includes(String(c.eventName)) || !("toolInput" in c) || s.plan !== null) return false;
    } else return false;
    if (!validDecision(v.decision,v.body) || (v.origin === "hook" ? v.workflowId !== null : !id(v.workflowId))) return false;
    seen.add(v.requestId); return true;
  });
}
export function validateCursorAnswers(questions: CursorQuestion[], answers: CursorAnswer[]): string | null {
  if (answers.length !== questions.length || new Set(answers.map(a => a.questionId)).size !== questions.length) return "请逐项回答问题。";
  for (const q of questions) {
    const a = answers.find(a => a.questionId === q.questionId);
    if (!a) return "请逐项回答问题。";
    if (a.skipped) { if (!q.allowSkip || a.text !== null || a.selectedOptionIds.length) return "此问题不能跳过。"; continue; }
    if (new Set(a.selectedOptionIds).size !== a.selectedOptionIds.length || (!q.multiSelect && a.selectedOptionIds.length > 1)
      || a.selectedOptionIds.some(id => !q.options.some(o => o.optionId === id))) return "选项已失效，请重新选择。";
    if ((!q.allowFreeText && a.text !== null) || [...(a.text ?? "")].length > q.maxTextLength) return "回答文字超出允许长度。";
    if (q.required && !a.selectedOptionIds.length && !a.text?.trim()) return "请完成必答问题。";
  }
  return null;
}
export const cursorRequestLabel = (r: CursorRequest) => ({ permission: "工具审批", question: "用户作答", plan: "计划审阅" }[r.body.kind]);
export const cursorRequestIdentity = (r: CursorRequest) => [r.requestId, r.submission.appEpoch, r.submission.requestVersion,
  r.submission.argumentsDigest, r.submission.plan?.planId, r.submission.plan?.revision, r.submission.plan?.contentDigest].join(":");
export function cursorRequestStatus(r: CursorRequest): string {
  if (r.transport === "written") return r.body.kind === "permission" ? "决定已发送；工具执行结果未确认。" : r.body.kind === "question" ? "回答已交付。" : "审阅结果已交付。";
  if (r.transport === "outcomeUnknown") return "决定送达情况未知，不会自动重发。";
  if (r.transport === "deliveryFailed") return "决定未送达。";
  if (r.phase === "pending") return "等待人工决定";
  if (r.phase === "decided") return "决定已记录，等待调用方领取。";
  if (r.body.kind === "permission" && ["expired","disconnected"].includes(r.phase)) return "审批已结束；故障或超时可能放行，执行结果未确认。";
  return ({ cancelled: "已取消", expired: "已过期", superseded: "已被新版本替换", disconnected: "连接中断" } as Record<string,string>)[r.phase] ?? r.phase;
}
