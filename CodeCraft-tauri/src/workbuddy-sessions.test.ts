import { describe, expect, it } from "vitest";
import {
  workBuddyInteractionStatusLabel,
  workBuddyPendingInteractions,
  workBuddyRequestKindLabel,
  workBuddySessionKey,
  workBuddyStageLabel,
  workBuddyVisualStatus,
  workBuddyMetadata,
  workBuddySessionTitle,
  type WorkBuddySession,
  type WorkBuddyInteraction,
} from "./workbuddy-sessions";

const session: WorkBuddySession = {
  id: "session-1",
  title: "Review authentication flow",
  titleSource: "native",
  workbuddyVersion: "5.5.3",
  cliVersion: "2.137.1",
  pluginInstanceId: "plugin-1",
  processInstanceId: "process-1",
  cwdHash: "cwd-hash",
  transcriptHash: null,
  permissionMode: "default",
  stage: "waitingForInput",
  currentTool: "AskUserQuestion",
  lastPrompt: null,
  eventCount: 2,
  startedAt: 1,
  updatedAt: 2,
  endedAt: null,
  pendingCount: 1,
  activities: [],
  outputs: [],
};

describe("WorkBuddy read-only session helpers", () => {
  const request = (requestKey: string, kind: WorkBuddyInteraction["kind"], capturedAt: number): WorkBuddyInteraction => ({
    requestKey, kind, capturedAt, sessionId: session.id, status: "pending",
    nativeRequestId: requestKey, toolName: null, toolInput: null, summary: "Waiting",
    questions: [], plan: null, payloadHash: "hash", expiresAt: 100, answerable: false, reason: "只读提醒",
  });

  it("retains all three reminders in one session until their own results arrive", () => {
    const interactions = [request("question", "question", 1), request("plan", "plan", 2), request("tool", "tool", 3)];
    expect(workBuddyPendingInteractions([session], interactions).map(({interaction}) => interaction.requestKey))
      .toEqual(["question", "plan", "tool"]);
    interactions[0].status = "completed";
    expect(workBuddyPendingInteractions([session], interactions).map(({interaction}) => interaction.requestKey))
      .toEqual(["plan", "tool"]);
    interactions[1].status = "denied";
    interactions[2].status = "failed";
    expect(workBuddyPendingInteractions([session], interactions)).toEqual([]);
  });

  it("does not treat an ordinary tool call as an approval or revive ended sessions", () => {
    const tool = {...request("tool", "tool", 1), status: "unavailable" as const};
    expect(workBuddyPendingInteractions([session], [tool])).toEqual([]);
    const question = request("question", "question", 2);
    expect(workBuddyPendingInteractions([{...session, endedAt: 3}], [question])).toEqual([]);
    expect(workBuddyPendingInteractions([], [question])).toEqual([]);
  });

  it("maps protocol stages to the existing session status vocabulary", () => {
    expect(workBuddyVisualStatus("toolRunning")).toBe("working");
    expect(workBuddyVisualStatus("waitingForInput")).toBe("waiting");
    expect(workBuddyVisualStatus("toolFailed")).toBe("toolFailed");
    expect(workBuddyVisualStatus("stopped")).toBe("stopped");
    expect(workBuddyStageLabel("unknown")).toBe("unknown");
  });

  it("keeps the internal key stable and labels observed interaction state", () => {
    expect(workBuddySessionKey(session)).toBe("workbuddy:session-1");
    expect(workBuddyRequestKindLabel("question")).toBe("问题");
    expect(workBuddyInteractionStatusLabel("unavailable")).toBe("只读观察");
    expect(workBuddyInteractionStatusLabel("pending")).toBe("等待审批");
  });

  it("distinguishes a native completion from a CodeCraft submission", () => {
    expect(workBuddyInteractionStatusLabel("completed")).toBe("WorkBuddy 已完成");
    expect(workBuddyInteractionStatusLabel("failed")).toBe("WorkBuddy 执行失败");
  });

  it("keeps the opaque integration key separate from the visible native id", () => {
    const observed = { ...session, id: "wbs_random", workbuddySessionId: "native-id" };
    expect(workBuddySessionTitle(observed)).toBe("Review authentication flow");
    expect(workBuddySessionKey(observed)).toBe("workbuddy:wbs_random");
    const metadata = Object.fromEntries(workBuddyMetadata(observed, false));
    expect(metadata["本地桥"]).toBe("连接异常");
    expect(metadata["身份状态"]).toContain("隔离观察");
    expect(metadata["来源"]).toContain("未知");
  });

  it("uses a readable compatibility fallback without coupling the title to the active tool", () => {
    expect(workBuddySessionTitle({
      ...session,
      title: "",
      titleSource: "fallback",
      lastPrompt: "Investigate the login failure\nThen run the tests",
    })).toBe("Investigate the login failure");
    expect(workBuddySessionTitle({
      ...session,
      title: "",
      titleSource: "fallback",
      lastPrompt: null,
      workbuddySessionId: "123456789012345678901234567890",
    })).toBe("WorkBuddy 会话 · 123456789012…34567890");
  });
});
