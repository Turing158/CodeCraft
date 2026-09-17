import { describe, expect, it } from "vitest";
import { mergeSnapshot } from "./snapshot";
import type { WorkBuddySession, WorkBuddySnapshot } from "../../src/workbuddy-sessions";

const session: WorkBuddySession = {
  id: "wbs_1", workbuddySessionId: "same-native-id", workbuddyVersion: "37.10.3-24",
  title: "修复登录流程", titleSource: "native",
  cliVersion: "2.137.1", pluginInstanceId: "plugin", processInstanceId: "10:1",
  cwdHash: "safe-hash", transcriptHash: null, permissionMode: "default",
  stage: "waitingForInput", currentTool: "ExitPlanMode", lastPrompt: null,
  eventCount: 1, startedAt: 1, updatedAt: 2, endedAt: null, pendingCount: 0,
  activities: [], outputs: [],
};
const snapshot: WorkBuddySnapshot = {
  connected: true, integrationError: null, version: 1, sessions: [session],
  observedEventCount: 1, unknownEventCount: 0,
  capabilities: { canObserve: true, canApproveTools: false, canAnswerQuestions: false, canApprovePlans: false, canStreamOutput: false, protocolFrozen: false, reason: "read-only" },
  interactions: [{ requestKey: "wbk_1", sessionId: "wbs_1", kind: "plan", status: "unavailable", nativeRequestId: null, toolName: "ExitPlanMode", toolInput: {}, summary: "plan source unavailable", questions: [], plan: null, payloadHash: "hash", capturedAt: 1, expiresAt: 10, answerable: false, reason: "unverified" }],
};

describe("LAN WorkBuddy read-only snapshots", () => {
  it("keeps separate integration sessions despite identical native ids", () => {
    const view = mergeSnapshot({ workbuddy: { ...snapshot, sessions: [session, { ...session, id: "wbs_2", processInstanceId: "11:2" }] } });
    expect(view.entries).toHaveLength(2);
    expect(view.entries[0].key).not.toBe(view.entries[1].key);
    expect(view.entries[0].title).toBe("修复登录流程");
    expect(view.entries[0].cwd).toBe("目录摘要 safe-hash");
  });
  it("never enables plan submission from a claimed capability", () => {
    const view = mergeSnapshot({ workbuddy: { ...snapshot, capabilities: { ...snapshot.capabilities, protocolFrozen: true, canApprovePlans: true } } });
    expect(view.entries[0].pending?.readOnly).toBe(true);
    expect(view.entries[0].pending?.plan?.plan).toContain("source unavailable");
  });
  it("does not resurrect completed or superseded requests", () => {
    for (const status of ["completed", "failed", "superseded", "timeout"] as const) {
      const view = mergeSnapshot({ workbuddy: { ...snapshot, interactions: [{ ...snapshot.interactions[0], status }] } });
      expect(view.entries[0].pending).toBeNull();
    }
  });
  it("shows native permissions but does not mistake ordinary tools for approvals", () => {
    for (const status of ["pending", "unavailable"] as const) {
      const view = mergeSnapshot({workbuddy: {...snapshot, interactions: [{...snapshot.interactions[0], kind: "tool", status}]}});
      expect(view.entries[0].pending?.kind ?? null).toBe(status === "pending" ? "permission" : null);
    }
  });
});
