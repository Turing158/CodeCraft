import { expect, it } from "vitest";
import { emptyCursorSnapshot, readCursorSnapshot, CursorSnapshotTracker, toCursorUnifiedSession, type CursorSession } from "./cursor-sessions";
import { isCursorSession, isTraeSession, unifiedSessionKey, unifiedSessionSource } from "./unified-sessions";
import { mergeSnapshot } from "../web/lan/snapshot";
import { createCursorInteractionPreview } from "./cursor-preview";
import type { CursorDecision, CursorRequest } from "./cursor-interactions";

const session: CursorSession = { source: "cursor", key: "a".repeat(64), conversationId: "c", title: "Cursor captured session", updatedAt: 100,
  activity: "interrupted", hasCurrentRequest: false, readOnly: true, truncated: false,
  items: [{ kind: "tool", generation: "g", callId: "call", name: "Read", outcome: "started" },
    { kind: "response", generation: "g", text: "<script>captured text</script>", truncated: false }] };
const snapshot = (appEpoch = "epoch-1", version = 1) => ({ ...emptyCursorSnapshot(), appEpoch, version, sessions: [session], connected: true,
  hook: { enabled: true, filesInstalled: true, detected: true, cursorVersion: "3.21.16", cursorExecutable: "D:/Cursor.exe", installPath: "D:/.cursor", configRevision: 1, state: "installed", error: null } });

it("keeps Cursor identity separate and recovered activity cannot look like a running tool", () => {
  const s = toCursorUnifiedSession(session);
  expect(isCursorSession(s)).toBe(true);
  expect(isTraeSession(s)).toBe(false);
  expect(unifiedSessionSource(s)).toBe("cursor");
  expect(unifiedSessionKey(s)).toBe(`cursor:${session.key}`);
  expect(s.activities[0].status).toBe("unknown");
  expect(s.outputs[0].text).toBe("<script>captured text</script>");
  expect(s.question).toBeNull();
  expect(s.permission).toBeNull();
});
it("rejects malformed records, deduplicates keys and fails closed on unsupported future request shapes", () => {
  const v = readCursorSnapshot({ ...snapshot(), sessions: [session, session, { ...session, updatedAt: Infinity }],
    capabilities: { askUser: true, nativeShared: true }, requests: [{ kind: "permission" }] });
  expect(v.sessions).toHaveLength(1);
  expect(v.requests).toEqual([]);
  expect(v.capabilities.askUser).toBe(false);
  expect(readCursorSnapshot({ ...snapshot(), schemaVersion: 2 }).connected).toBe(false);
});
it("ignores out of order versions and retired epochs while preserving records offline", () => {
  const tracker = new CursorSnapshotTracker();
  tracker.accept(snapshot("a", 9));
  expect(tracker.accept(snapshot("a", 8)).version).toBe(9);
  tracker.accept(snapshot("b", 1));
  expect(tracker.accept(snapshot("a", 10)).appEpoch).toBe("b");
  expect(tracker.accept(emptyCursorSnapshot())).toMatchObject({ appEpoch: "b", connected: false, sessions: [session] });
});
it("desktop and LAN retain the same session key, output, unknown result and read-only boundary", () => {
  const lan = mergeSnapshot({ cursor: snapshot(), allowApprovals: true });
  const entry = lan.entries.find(e => e.source === "cursor")!;
  const desktop = toCursorUnifiedSession(session);
  expect(entry.key).toBe(unifiedSessionKey(desktop));
  expect(entry.outputs).toEqual(desktop.outputs);
  expect(entry.activities).toEqual(desktop.activities);
  expect(entry.pending).toBeNull();
  expect(lan.integrations.find(i => i.source === "cursor")?.hookInstalled).toBe(true);
});

it("uninstall hides retained sessions even if an older snapshot still reports connected", () => {
  const previous = snapshot();
  const removed = { ...previous, hook: { ...previous.hook, enabled: false, filesInstalled: false } };
  expect(readCursorSnapshot(removed)).toMatchObject({ connected: false, sessions: [], requests: [] });
  const lan = mergeSnapshot({ cursor: previous, integrations: [{ id: "cursor", hookInstalled: false }] });
  expect(lan.entries).toEqual([]);
  expect(lan.integrations[0]).toMatchObject({ connected: false, hookInstalled: false, sessionCount: 0 });
  expect(readCursorSnapshot({ ...previous, connected: false }).sessions).toHaveLength(1);
});

it("fills the shared transcript with this session's questions, answers, plans and feedback", () => {
  const preview = createCursorInteractionPreview();
  const requests: CursorRequest[] = preview.requests.map((r, i) => ({
    ...r, sessionKey: session.key, createdAt: 10 + i * 10, updatedAt: 15 + i * 10,
    actionable: false, phase: "decided", transport: "written",
    decision: r.body.kind === "question" ? { value: { kind: "answered", answers: [
      { questionId: "target", selectedOptionIds: ["dev"], text: "保留中文回答", skipped: false },
      { questionId: "checks", selectedOptionIds: [], text: null, skipped: true },
    ] } } : r.body.kind === "plan" ? { value: { kind: "plan", action: "revise", feedback: "先检查输入文件" } } : null,
  }));
  const foreign = { ...requests[0], requestId: "another-session", sessionKey: "b".repeat(64) };
  const adapted = toCursorUnifiedSession(session, [...requests, foreign]);
  expect(adapted.outputs.map(o => o.role)).toEqual(["assistant", "user", "assistant", "user", "assistant"]);
  expect(adapted.outputs[0].text).toContain("选择目标环境");
  expect(adapted.outputs[1].text).toContain("开发环境\n\n保留中文回答");
  expect(adapted.outputs[1].text).toContain("已跳过");
  expect(adapted.outputs[2].text).toContain("合成计划：验证独立审批");
  expect(adapted.outputs[3].text).toBe("要求修改\n\n先检查输入文件");
  expect(new Set(adapted.outputs.map(o => o.id)).size).toBe(adapted.outputs.length);
  expect(adapted.outputs.some(o => o.id.includes("another-session"))).toBe(false);
  expect(adapted.pendingReviewId).toBeNull();
});

it("keeps approval delivery separate from native execution and retains stable tool IDs", () => {
  const permission = createCursorInteractionPreview().requests[2];
  const request: CursorRequest = { ...permission, sessionKey: session.key, actionable: false, phase: "decided", transport: "written",
    decision: { value: { kind: "permission", action: "allowOnce" } },
    body: { kind: "permission", content: { eventName: "preToolUse", toolName: "Read", toolInput: { path: "说明.md" } } } };
  const adapted = toCursorUnifiedSession(session, [request]);
  expect(adapted.activities).toHaveLength(2);
  const approval = adapted.activities.find(a => a.id === `review:${request.requestId}`)!;
  expect(approval).toMatchObject({ status: "unknown", tool: "Read · 工具审批" });
  expect(approval.details).toContain('"path": "说明.md"');
  expect(approval.details).toContain("工具执行结果未确认");
  const reordered = toCursorUnifiedSession({ ...session, items: [...session.items].reverse() }, [request]);
  expect(reordered.activities.map(a => a.id)).toEqual(adapted.activities.map(a => a.id));
});

it("uses the same review history and pending status in desktop and LAN details", () => {
  const preview = createCursorInteractionPreview();
  const target = preview.sessions[2];
  const desktop = toCursorUnifiedSession(target, preview.requests);
  const lan = mergeSnapshot({ cursor: preview, allowApprovals: false });
  const entry = lan.entries.find(e => e.sessionId === target.key)!;
  expect(desktop.status).toBe("attention");
  expect(desktop.pendingReviewId).toBe(preview.requests[2].requestId);
  expect(entry.statusLabel).toBe("需要处理");
  expect(entry.activities).toEqual(desktop.activities);
  expect(entry.pending?.readOnly).toBe(true);
  const question = toCursorUnifiedSession(preview.sessions[0], preview.requests);
  expect(question.status).toBe("waiting");
  expect(question.outputs[0].text).toContain("选择目标环境");
});

it.each(["question", "plan", "permission"] as const)("releases %s waiting on acknowledgement before delivery, despite in-flight polls", kind => {
  const preview = createCursorInteractionPreview();
  const request = preview.requests.find(r => r.body.kind === kind)!;
  const decision: CursorDecision = kind === "question" ? { kind: "questionCancelled" }
    : kind === "plan" ? { kind: "plan", action: "approved", feedback: "" }
    : { kind: "permission", action: "allowOnce" };
  const tracker = new CursorSnapshotTracker();
  tracker.accept(preview);
  const accepted = tracker.recordDecision(request, decision);
  const current = accepted.requests.find(r => r.requestId === request.requestId)!;
  expect(current).toMatchObject({ phase: "decided", actionable: false, transport: "notStarted", decision: { value: decision } });
  const session = accepted.sessions.find(s => s.key === request.sessionKey)!;
  expect(session.hasCurrentRequest).toBe(false);
  expect(toCursorUnifiedSession(session, accepted.requests).pendingReviewId).toBeNull();
  expect(toCursorUnifiedSession(session, accepted.requests).status).not.toBe("waiting");
  for (const version of [preview.version, preview.version + 1]) {
    const polled = tracker.accept({ ...preview, version });
    expect(polled.requests.find(r => r.requestId === request.requestId)?.actionable).toBe(false);
    expect(polled.sessions.find(s => s.key === request.sessionKey)?.hasCurrentRequest).toBe(false);
  }
  const delivered = tracker.accept({ ...accepted, version: preview.version + 2,
    requests: accepted.requests.map(r => r.requestId === request.requestId ? { ...r, transport: "written" } : r) });
  expect(delivered.requests.find(r => r.requestId === request.requestId)?.transport).toBe("written");
});

it("keeps a queued review waiting and never applies an old acknowledgement to a revised form or restarted broker", () => {
  const preview = createCursorInteractionPreview();
  const old = preview.requests[1];
  preview.requests[0].sessionKey = old.sessionKey;
  const tracker = new CursorSnapshotTracker();
  tracker.accept(preview);
  const decision: CursorDecision = { kind: "plan", action: "approved", feedback: "" };
  expect(tracker.recordDecision(old, decision).sessions.find(s => s.key === old.sessionKey)?.hasCurrentRequest).toBe(true);
  const revised = { ...old, submission: { ...old.submission, requestVersion: old.submission.requestVersion + 1 } };
  tracker.accept({ ...preview, version: 2, requests: [revised] });
  expect(tracker.recordDecision(old, decision).requests[0].actionable).toBe(true);
  tracker.accept({ ...preview, appEpoch: "new-epoch", version: 1,
    requests: [{ ...old, submission: { ...old.submission, appEpoch: "new-epoch" } }] });
  expect(tracker.recordDecision(old, decision).requests[0].actionable).toBe(true);
});
