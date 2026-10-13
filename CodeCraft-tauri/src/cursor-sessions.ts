import type { ClaudeSession, ClaudeActivity, ClaudeOutputEntry } from "./claude-sessions";
import { currentLocale, translateText } from "./i18n";
import { cursorRequestIdentity, cursorRequestStatus, readCursorRequests, type CursorDecision, type CursorRequest } from "./cursor-interactions";
import { cursorToolDisplay } from "./cursor-tool-display";

export type CursorActivity = "working" | "stopped" | "unknown" | "interrupted";
export type CursorOutcome = "started" | "completedObserved" | "failedObserved" | "cancelledObserved" | "unknown";
export type CursorItem =
  | { kind: "response"; generation: string | null; text: string; truncated: boolean }
  | { kind: "tool"; generation: string | null; callId: string; name: string; outcome: CursorOutcome }
  | { kind: "subagent"; generation: string | null; agentId: string; parentId: string | null; outcome: CursorOutcome };
export interface CursorSession {
  key: string; source: "cursor"; conversationId: string; title: string; updatedAt: number;
  activity: CursorActivity; items: CursorItem[]; truncated: boolean; hasCurrentRequest: boolean; readOnly: true;
  interactionOrigin?: "hook" | "independentMcp";
}
export interface CursorHookStatus {
  enabled: boolean; filesInstalled: boolean; detected: boolean; cursorVersion: string | null;
  cursorExecutable: string | null; installPath: string; configRevision: number; state: string; error: string | null;
}
export interface CursorSnapshot {
  schemaVersion: 1; source: "cursor"; captureScope: "observedOnly"; appEpoch: string; version: number;
  connected: boolean; integrationError: string | null; sessions: CursorSession[]; requests: CursorRequest[];
  interactionProtocol?: 1;
  capabilities: { hookOnly: boolean; nativeShared: boolean; askUser: boolean; reviewPlan: boolean; reason: string; blockers: string[]; interactionMode?: "independent" };
  hook: CursorHookStatus | null; lastObservationAt: number | null;
}
export const CURSOR_NOTICE = "CodeCraft 独立工具审批、问答与计划审阅；不与 Cursor 原生审批同步，计划结果不拦截未关联的原生工具。";
export const emptyCursorSnapshot = (): CursorSnapshot => ({ schemaVersion: 1, source: "cursor", captureScope: "observedOnly",
  appEpoch: "", version: 0, connected: false, integrationError: null, sessions: [], requests: [], hook: null, lastObservationAt: null,
  capabilities: { hookOnly: false, nativeShared: false, askUser: false, reviewPlan: false, reason: CURSOR_NOTICE, blockers: [] } });
const record = (v: unknown): v is Record<string, unknown> => !!v && typeof v === "object" && !Array.isArray(v);
const text = (v: unknown, max = 32768): v is string => typeof v === "string" && v.length <= max;
const time = (v: unknown): v is number => typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
const outcome = (v: unknown): v is CursorOutcome => ["started", "completedObserved", "failedObserved", "cancelledObserved", "unknown"].includes(String(v));
const item = (v: unknown): v is CursorItem => record(v) && (v.generation === null || text(v.generation, 128)) && (
  (v.kind === "response" && text(v.text) && typeof v.truncated === "boolean") ||
  (v.kind === "tool" && text(v.callId, 128) && text(v.name, 2048) && outcome(v.outcome)) ||
  (v.kind === "subagent" && text(v.agentId, 128) && (v.parentId === null || text(v.parentId, 128)) && outcome(v.outcome)));
const session = (v: unknown): v is CursorSession => record(v) && v.source === "cursor" && text(v.key, 64) && /^[a-f0-9]{64}$/.test(v.key)
  && text(v.conversationId, 128) && text(v.title, 4096) && time(v.updatedAt)
  && ["working", "stopped", "unknown", "interrupted"].includes(String(v.activity))
  && Array.isArray(v.items) && v.items.length <= 256 && v.items.every(item)
  && typeof v.truncated === "boolean" && typeof v.hasCurrentRequest === "boolean" && v.readOnly === true;

export function readCursorSnapshot(value: unknown): CursorSnapshot {
  const empty = emptyCursorSnapshot();
  if (!record(value) || value.schemaVersion !== 1 || value.source !== "cursor" || value.captureScope !== "observedOnly"
    || !text(value.appEpoch, 36) || !time(value.version) || !Array.isArray(value.sessions) || value.sessions.length > 1000) return empty;
  const sessions = value.sessions.filter(session);
  const unique = new Set<string>();
  const hook = record(value.hook) && typeof value.hook.enabled === "boolean" && typeof value.hook.filesInstalled === "boolean"
    && typeof value.hook.detected === "boolean" && time(value.hook.configRevision)
    && text(value.hook.state, 64) && text(value.hook.installPath, 32768) ? value.hook as unknown as CursorHookStatus : null;
  const supported = value.interactionProtocol === 1 && record(value.capabilities) && value.capabilities.interactionMode === "independent";
  const caps = supported ? value.capabilities as Record<string,unknown> : {};
  const requests = supported ? readCursorRequests(value.requests,value.appEpoch) : [];
  const uninstalled = hook !== null && (!hook.enabled || !hook.filesInstalled);
  return { ...empty, appEpoch: value.appEpoch, version: value.version,
    connected: value.connected === true && hook?.enabled === true && hook.filesInstalled,
    integrationError: text(value.integrationError, 256) ? value.integrationError : null, hook,
    lastObservationAt: time(value.lastObservationAt) ? value.lastObservationAt : null,
    sessions: uninstalled ? [] : sessions.filter(s => !unique.has(s.key) && !!unique.add(s.key)),
    interactionProtocol: supported ? 1 : undefined, requests: uninstalled ? [] : requests,
    capabilities: { ...empty.capabilities, hookOnly: caps.hookOnly === true, askUser: caps.askUser === true, reviewPlan: caps.reviewPlan === true, nativeShared: false, interactionMode: supported ? "independent" : undefined },
  };
}
export class CursorSnapshotTracker {
  private current = emptyCursorSnapshot();
  private retired = new Set<string>();
  private decisions = new Map<string, { decision: CursorDecision; updatedAt: number }>();
  accept(value: unknown): CursorSnapshot {
    const next = readCursorSnapshot(value);
    if (next.appEpoch && this.retired.has(next.appEpoch)) return this.current;
    if (next.appEpoch && next.appEpoch === this.current.appEpoch && next.version <= this.current.version) return this.current;
    if (next.appEpoch !== this.current.appEpoch) {
      if (!next.appEpoch) return { ...this.current, connected: false, integrationError: next.integrationError };
      if (this.current.appEpoch) this.retired.add(this.current.appEpoch);
    }
    for (const key of this.decisions.keys()) {
      if (!next.requests.some(r => cursorRequestIdentity(r) === key && r.phase === "pending")) this.decisions.delete(key);
    }
    this.current = this.applyDecisions(next);
    return this.current;
  }
  /** A successful submission acknowledges the human decision, independently of
   * result delivery or tool execution. Older polling responses cannot undo it. */
  recordDecision(request: CursorRequest, decision: CursorDecision): CursorSnapshot {
    const current = this.current.requests.find(r => cursorRequestIdentity(r) === cursorRequestIdentity(request));
    if (!current || current.phase !== "pending" || current.submission.appEpoch !== this.current.appEpoch) return this.current;
    this.decisions.set(cursorRequestIdentity(current), { decision, updatedAt: Math.max(current.updatedAt, Date.now()) });
    this.current = this.applyDecisions(this.current);
    return this.current;
  }
  private applyDecisions(snapshot: CursorSnapshot): CursorSnapshot {
    const changed = new Map<string, number>();
    const requests = snapshot.requests.map(r => {
      const accepted = this.decisions.get(cursorRequestIdentity(r));
      if (!accepted || r.phase !== "pending") return r;
      changed.set(r.sessionKey, Math.max(changed.get(r.sessionKey) ?? 0, accepted.updatedAt));
      return { ...r, actionable: false, phase: "decided", decision: { value: accepted.decision }, updatedAt: accepted.updatedAt };
    });
    if (!changed.size) return snapshot;
    return { ...snapshot, requests, sessions: snapshot.sessions.map(s => {
      const updatedAt = changed.get(s.key);
      if (updatedAt === undefined) return s;
      const pending = requests.some(r => r.sessionKey === s.key && r.actionable);
      return { ...s, hasCurrentRequest: pending, updatedAt: Math.max(s.updatedAt, updatedAt),
        activity: !pending && s.interactionOrigin === "independentMcp" ? "stopped" : s.activity };
    }) };
  }
}
const t = (s: string) => translateText(s, currentLocale());
export const cursorActivityLabel = (activity: CursorActivity) => t({ working: "工作中", stopped: "停止", unknown: "状态未知", interrupted: "连接中断" }[activity]);
export interface CursorUnifiedSession extends ClaudeSession {
  source: "cursor"; sessionKey: string; detailNotice: string; cursorActivity: CursorActivity;
  pendingReviewId: string | null;
}

function reviewOutputs(request: CursorRequest): ClaudeOutputEntry[] {
  const { body, decision, requestId, createdAt, updatedAt } = request;
  const entries: ClaudeOutputEntry[] = [];
  const append = (part: string, text: string, role: "user" | "assistant", capturedAt: number) => {
    entries.push({ id: `review:${requestId}:${part}`, text, role, capturedAt });
  };
  if (body.kind === "question") {
    append("questions", body.content.questions.map((q, i) => [
      `### ${t("问题")} ${i + 1}`, q.prompt,
      q.options.map(option => `- ${option.label}`).join("\n"),
    ].filter(Boolean).join("\n\n")).join("\n\n"), "assistant", createdAt);
    if (decision?.value.kind === "answered") {
      const answers = decision.value.answers;
      append("answers", body.content.questions.map((q, i) => {
        const answer = answers.find(a => a.questionId === q.questionId);
        if (!answer) return "";
        const text = answer.skipped ? t("已跳过") : [
          ...answer.selectedOptionIds.map(id => q.options.find(option => option.optionId === id)?.label ?? id),
          answer.text,
        ].filter(Boolean).join("\n\n");
        return `### ${t("问题")} ${i + 1}\n\n${text}`;
      }).filter(Boolean).join("\n\n"), "user", updatedAt);
    } else if (decision?.value.kind === "questionCancelled") {
      append("cancelled", t("问题已取消"), "user", updatedAt);
    }
  } else if (body.kind === "plan") {
    const plan = body.content;
    const states: Record<string, string> = { pending: "待开始", inProgress: "进行中", completed: "已完成", cancelled: "已取消" };
    append("plan", [
      `### ${plan.title}`, plan.overview, plan.markdown,
      ...plan.phases.map(phase => `#### ${phase.title}\n\n${phase.description}`),
      plan.todos.map(todo => `- [${t(states[todo.status] ?? todo.status)}] ${todo.description}`).join("\n"),
    ].filter(Boolean).join("\n\n"), "assistant", createdAt);
    if (decision?.value.kind === "plan") {
      const value = decision.value;
      append("feedback", [t({ approved: "已批准计划", revise: "要求修改", rejected: "已拒绝计划", cancelled: "已取消审阅" }[value.action]), value.feedback].filter(Boolean).join("\n\n"), "user", updatedAt);
    }
  }
  return entries;
}

/** Feed Cursor's captured events and independent reviews into the shared detail
 * sections. Session keys are the only association; names/text cannot join calls. */
export function toCursorUnifiedSession(s: CursorSession, requests: readonly CursorRequest[] = []): CursorUnifiedSession {
  const reviews = requests.filter(r => r.sessionKey === s.key).sort((a, b) => a.createdAt - b.createdAt || a.requestId.localeCompare(b.requestId));
  const pending = reviews.find(r => r.actionable);
  const activities: ClaudeActivity[] = s.items.flatMap(entry => {
    if (entry.kind === "response") return [];
    const running = entry.outcome === "started" && s.activity === "working";
    return [{ id: `native:${entry.generation ?? ""}:${entry.kind}:${entry.kind === "tool" ? entry.callId : entry.agentId}`, tool: entry.kind === "tool" ? entry.name : `Subagent ${entry.agentId}`,
      summary: t(running ? "等待工具返回结果" : entry.outcome === "completedObserved" ? "已观察到完成事件" : entry.outcome === "failedObserved" ? "已观察到失败事件" : entry.outcome === "cancelledObserved" ? "已取消" : "未收到工具结果，无法确认执行是否成功"),
      status: running ? "running" : entry.outcome === "completedObserved" ? "completed" : entry.outcome === "failedObserved" ? "failed" : "unknown",
      startedAt: s.updatedAt, updatedAt: s.updatedAt }];
  });
  for (const r of reviews) {
    if (r.body.kind !== "permission") continue;
    const display = cursorToolDisplay(r.body.content, t);
    const decision = r.decision?.value;
    const state = t(cursorRequestStatus(r));
    const result = decision?.kind === "permission" ? t(decision.action === "allowOnce" ? "允许本次" : "拒绝本次") : "";
    activities.push({
      id: `review:${r.requestId}`, tool: `${r.body.content.toolName} · ${t("工具审批")}`,
      summary: [display.summary, result || state].join(" · "),
      details: [display.details, result, state].filter(Boolean).join("\n\n"),
      // Delivery of a human decision does not confirm native tool execution.
      status: "unknown", startedAt: r.createdAt, updatedAt: r.updatedAt,
    });
  }
  activities.sort((a, b) => a.startedAt - b.startedAt);
  const outputs: ClaudeOutputEntry[] = s.items.flatMap((entry, index) => entry.kind === "response"
    ? [{ id: `${s.key}:${index}`, text: entry.text, role: "assistant" as const, capturedAt: s.updatedAt }] : []);
  outputs.push(...reviews.flatMap(reviewOutputs));
  outputs.sort((a, b) => (a.capturedAt ?? 0) - (b.capturedAt ?? 0));
  return { source: "cursor", sessionKey: s.key, id: s.key, title: s.title, cursorActivity: s.activity,
    status: pending ? pending.body.kind === "question" ? "waiting" : "attention" : s.hasCurrentRequest ? "waiting" : s.activity === "working" ? "working" : s.activity === "unknown" ? "idle" : "stopped",
    startedAt: Math.min(s.updatedAt, reviews[0]?.createdAt ?? s.updatedAt), updatedAt: s.updatedAt,
    question: null, permission: null, plan: null, activities, outputs, pendingReviewId: pending?.requestId ?? null,
    detailNotice: t(s.interactionOrigin === "independentMcp" ? "此记录属于 CodeCraft 独立工作流，不对应已确认的 Cursor 原生对话。" : s.truncated ? "仅显示近期采集记录，较早或过长的内容已截断。" : "仅显示 CodeCraft 已采集的记录，不包含接入前的完整历史。") };
}
