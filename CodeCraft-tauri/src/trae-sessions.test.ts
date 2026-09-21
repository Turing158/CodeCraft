import { describe, expect, it } from "vitest";
import { traeReviews, traeQuestionRequest, traePlanRequest, TraeReviewQueue, canDecide, emptyTraeSnapshot, readTraeSnapshot, TraeDecisionIds, validateTraeAnswers, toTraeUnifiedSession, type QuestionInput, type TraeRequest } from "./trae-sessions";
import { isGeminiSession, isOpenCodeSession, unifiedSessionSource } from "./unified-sessions";
import { mergeSnapshot } from "../web/lan/snapshot";
const input: QuestionInput = { schemaVersion: 1, questions: [{ questionId: "q", prompt: "Choose", kind: "single", options: [{ optionId: "a", label: "Same", description: null }, { optionId: "b", label: "Same", description: null }], required: true, allowText: false, minSelections: 1, maxSelections: 1 }] };
const request = (): TraeRequest => ({ target: { appEpoch: "epoch", sessionKey: "session", taskId: null, turnEpoch: 1, requestId: "request", requestVersion: 1 }, kind: "question", channel: "mcp", toolName: "codecraft_ask_user", toolUseId: "call", arguments: input as unknown as Record<string, unknown>, expiresAt: new Date(Date.now() + 60_000).toISOString(), state: "pending", plan: null, error: null });
describe("Trae shared desktop/LAN contract", () => {
  it("uses original IDs even when labels match", () => {
    expect(validateTraeAnswers(input, [{ questionId: "q", status: "answered", selectedOptionIds: ["b"], text: null }])).toBeNull();
    expect(validateTraeAnswers(input, [{ questionId: "q", status: "answered", selectedOptionIds: ["Same"], text: null }])).not.toBeNull();
  });
  it("supports pure text, Unicode scalar limits and explicit optional skipping", () => {
    const text: QuestionInput = { schemaVersion: 1, questions: [{ questionId: "text", prompt: "Text", kind: "text", options: [], required: true, allowText: true, minSelections: 0, maxSelections: 0 }] };
    expect(validateTraeAnswers(text, [{ questionId: "text", status: "answered", selectedOptionIds: [], text: "🦀".repeat(4096) }])).toBeNull();
    expect(validateTraeAnswers(text, [{ questionId: "text", status: "answered", selectedOptionIds: [], text: " " }])).not.toBeNull();
    expect(validateTraeAnswers(text, [{ questionId: "text", status: "skipped", selectedOptionIds: [], text: null }])).not.toBeNull();
    text.questions[0].required = false;
    expect(validateTraeAnswers(text, [{ questionId: "text", status: "skipped", selectedOptionIds: [], text: null }])).toBeNull();
  });
  it("rejects an old epoch/version, lost connection, disabled capability or expired request", () => {
    const snapshot = emptyTraeSnapshot(); const r: TraeRequest = { ...request(), kind: "permission", channel: "hook" }; snapshot.appEpoch = "epoch"; snapshot.connected = true; snapshot.capabilities.toolApproval = true; snapshot.requests = [r];
    expect(canDecide(snapshot, r)).toBe(true);
    expect(canDecide({ ...snapshot, appEpoch: "new" }, r)).toBe(false);
    expect(canDecide({ ...snapshot, connected: false }, r)).toBe(false);
    expect(canDecide(snapshot, { ...r, target: { ...r.target, requestVersion: 2 } })).toBe(false);
    r.expiresAt = new Date(0).toISOString(); expect(canDecide(snapshot, r)).toBe(false);
  });
  it("reuses a failed decision ID only for the same exact action and target", () => {
    const ids = new TraeDecisionIds(); const r = request(); const action = { kind: "cancel" as const, reason: null };
    expect(ids.for(r, action)).toBe(ids.for(r, action));
    expect(ids.for(r, action)).not.toBe(ids.for(r, { ...action, reason: "Changed" }));
    expect(ids.for(r, action)).not.toBe(ids.for({ ...r, target: { ...r.target, turnEpoch: 2 } }, action));
  });
  it("preserves explicit source through structural guards and LAN normalization", () => {
    const snapshot = emptyTraeSnapshot(); snapshot.sessions = [{ source: "trae", sessionKey: "s", sessionId: "native", installationId: "i", traeInstanceId: "t", cwd: "C:/workspace", workspaceRoots: ["C:/workspace"], title: "Trae task", status: "waitingForInput", turnEpoch: 0, taskId: null, nativeNotice: null, output: "", updatedAt: new Date().toISOString(), activities: [] }];
    const unified = { ...toTraeUnifiedSession(snapshot.sessions[0]), pendingInteractions: [], integrationStatus: "connected", pendingReviews: [] };
    expect(unifiedSessionSource(unified)).toBe("trae"); expect(isGeminiSession(unified)).toBe(false); expect(isOpenCodeSession(unified)).toBe(false);
    const lan = mergeSnapshot({ trae: snapshot, allowApprovals: false }); expect(lan.entries[0].key).toBe("trae:s"); expect(lan.entries[0].source).toBe("trae"); expect(lan.integrations.some(i => i.source === "trae")).toBe(true);
    expect(readTraeSnapshot(null).capabilities.mcpPlanReview).toBe(false);
  });
});


describe("Trae automatic native reviews", () => {
  const snapshot = () => {
    const s = emptyTraeSnapshot(); s.connected = true; s.appEpoch = "epoch";
    s.sessions = [{ source: "trae", sessionKey: "session", sessionId: "native", installationId: "i", traeInstanceId: "t", cwd: "C:/test", workspaceRoots: ["C:/test"], title: "Native", status: "waitingForInput", turnEpoch: 1, taskId: null, nativeNotice: null, nativeInteractions: [{ id: "native-q", kind: "question", toolUseId: "native-call", arguments: { questions: [{ question: "Which?", options: [{ label: "Same" }, { label: "Same" }] }] }, message: "", plan: null, documentPath: null, capturedAt: new Date().toISOString(), readOnly: true }], output: "", updatedAt: new Date().toISOString(), activities: [] }];
    return s;
  };
  it("keeps native questions read-only even when the client can write", () => {
    const s = snapshot(); s.requests = [request()];
    const reviews = traeReviews(s); expect(reviews).toHaveLength(1);
    const q = traeQuestionRequest(reviews[0]);
    expect(q.questions[0].readOnly).toBe(true); expect(q.questions[0].options).toHaveLength(2);
    expect(q.questions[0].allowOther).toBe(false);
    const lan = mergeSnapshot({ trae: s, allowApprovals: true }); expect(lan.entries[0].pending?.readOnly).toBe(true);
  });
  it("reveals a native event once, keeps the active review and advances when it ends", () => {
    const s = snapshot(); const queue = new TraeReviewQueue();
    const reviews = traeReviews(s); const first = queue.next(reviews)!; queue.mark(first.key);
    expect(queue.next(traeReviews(s))).toBeUndefined();
    expect(queue.next(reviews, first.key)?.key).toBe(first.key);
    expect(queue.next(traeReviews({ ...s, connected: false }))).toBeUndefined();
    s.appEpoch = "new"; expect(queue.next(traeReviews(s))).toBeDefined();
  });
  it("routes Hook permissions to actionable reviews and uses plan fallback without invented content", () => {
    const s = snapshot(); s.capabilities.toolApproval = true;
    s.requests = [{ ...request(), kind: "permission", channel: "hook" }];
    const reviews = traeReviews(s); expect(reviews[0].kind).toBe("permission"); expect(canDecide(s, reviews[0].request!)).toBe(true);
    s.sessions[0].nativeInteractions![0] = { ...s.sessions[0].nativeInteractions![0], kind: "plan", arguments: {} };
    expect(traePlanRequest(traeReviews(s)[1]).plan).toContain("未提供可可靠关联");
    const queue = new TraeReviewQueue(); queue.mark(reviews[0].key);
    s.requests[0].target.requestVersion++;
    expect(queue.next(traeReviews(s))?.kind).toBe("plan");
  });
  it("shares multi-turn conversation roles, timestamps and tool details with LAN", () => {
    const s = snapshot(); const session = s.sessions[0];
    session.cwd = String.raw`\\?\D:\Project`;
    session.startedAt = "2026-09-20T01:00:00Z";
    session.output = "legacy last reply";
    session.messages = [
      { id: "u1", role: "user", text: "First question", at: "2026-09-20T01:00:00Z" },
      { id: "a1", role: "assistant", text: "First reply", at: "2026-09-20T01:00:01Z" },
      { id: "u2", role: "user", text: "Second question", at: "2026-09-20T01:00:02Z" },
    ];
    session.activities = [
      { id: "1:call", tool: "Read", toolUseId: "call", status: "running", arguments: "path: D:\\Project\\file.txt", at: session.updatedAt },
      { id: "2:call", tool: "Read", toolUseId: "call", status: "unknown", arguments: "path: D:\\Project\\other.txt", at: session.updatedAt },
    ];
    const adapted = toTraeUnifiedSession(session);
    expect(adapted.cwd).toBe(String.raw`D:\Project`);
    expect(adapted.startedAt).toBe(Date.parse(session.startedAt));
    expect(adapted.outputs.map(o => [o.id, o.role, o.text])).toEqual(session.messages.map(m => [m.id, m.role, m.text]));
    expect(adapted.activities.map(a => a.status)).toEqual(["running", "unknown"]);
    expect(adapted.activities[0].summary).toContain("D:\\Project\\file.txt");
    expect(adapted.activities[1].summary).toContain("无法确认");
    const lan = mergeSnapshot({ trae: s });
    expect(lan.entries[0].outputs).toEqual(adapted.outputs);
    expect(lan.entries[0].activities).toEqual(adapted.activities);
    expect(lan.entries[0].detailNotice).toContain("已采集");
  });
  it("preserves legacy output and explains truncated history", () => {
    const session = snapshot().sessions[0]; session.output = "Old reply";
    expect(toTraeUnifiedSession(session).outputs).toMatchObject([{ text: "Old reply", role: "assistant" }]);
    session.historyTruncated = true;
    expect(toTraeUnifiedSession(session).detailNotice).toContain("截断");
  });
  it("shows the associated document body, path and source in the shared read-only plan", () => {
    const s = snapshot();
    s.sessions[0].nativeInteractions![0] = { ...s.sessions[0].nativeInteractions![0], kind: "plan", arguments: {},
      message: "Tool 'NotifyUser' requires user confirmation", plan: "# Approval plan\n\nKeep this content.",
      documentPath: "C:/test/.trae/documents/plan```draft.md", planSource: "session_file" };
    const review = traeReviews(s)[0];
    const plan = traePlanRequest(review).plan;
    expect(plan).toContain("# Approval plan");
    expect(plan).toContain("当前会话本轮");
    expect(plan).toContain("````\nC:/test/.trae/documents/plan```draft.md\n````");
    expect(plan).not.toContain("NotifyUser");
    const lan = mergeSnapshot({ trae: s, allowApprovals: true });
    expect(lan.entries[0].pending?.readOnly).toBe(true);
  });
  it("explains ambiguous and missing document associations without presenting a guessed body", () => {
    const s = snapshot();
    const native = s.sessions[0].nativeInteractions![0];
    native.kind = "plan"; native.arguments = {}; native.message = "Tool 'NotifyUser' requires user confirmation";
    expect(traePlanRequest(traeReviews(s)[0]).plan).toContain("未提供可可靠关联");
    native.planSource = "ambiguous";
    expect(traePlanRequest(traeReviews(s)[0]).plan).toContain("多个计划文档");
  });
});
