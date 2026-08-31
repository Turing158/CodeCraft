import { describe, expect, it } from "vitest";
import type { DshSession } from "../../src/dsh-sessions";
import { canAct, mergeSnapshot } from "./snapshot";

const session = (overrides: Partial<DshSession> = {}): DshSession => ({
  id: "session-1",
  pluginInstanceId: "plugin-1",
  bridgeInstanceId: "bridge-1",
  status: "waitingForApproval",
  title: "DeepSeek Harness session",
  cwd: "C:/work",
  startedAt: 1,
  updatedAt: 2,
  activities: [],
  outputs: [{ id: "output-1", text: "Reviewing the tool call" }],
  question: null,
  permission: {
    id: "permission-1",
    bridgeInstanceId: "bridge-1",
    pluginInstanceId: "plugin-1",
    sessionId: "session-1",
    callId: "call-1",
    toolName: "bash",
    summary: "npm test",
    cwd: "C:/work",
    canAlwaysAllow: false,
    capturedAt: 2,
  },
  plan: null,
  ...overrides,
});

describe("DSH LAN snapshot", () => {
  it("preserves bridge identity and one-shot permission capabilities", () => {
    const snapshot = mergeSnapshot({
      allowApprovals: true,
      dsh: {
        connected: true,
        integrationError: null,
        bridgeInstanceId: "bridge-1",
        sessions: [session()],
        instances: [],
      },
      integrations: [
        {
          id: "deepSeekHarness",
          name: "DeepSeek Harness",
          agentInstalled: true,
          hookInstalled: true,
        },
      ],
    });

    expect(snapshot.entries[0]).toMatchObject({
      key: "dsh:plugin-1:session-1",
      source: "dsh",
      pending: {
        kind: "permission",
        readOnly: false,
        dsh: {
          bridgeInstanceId: "bridge-1",
          pluginInstanceId: "plugin-1",
          sessionId: "session-1",
        },
        permission: { canAlwaysAllow: false },
      },
    });
    expect(canAct(snapshot.entries[0].pending, snapshot.allowApprovals)).toBe(
      true,
    );
    expect(snapshot.integrations[0]).toMatchObject({
      source: "dsh",
      connected: true,
      sessionCount: 1,
    });
  });
});
