import { describe, expect, it } from "vitest";
import {
  DSH_REVIEW_CAPABILITIES,
  dshPendingReview,
  dshQuestionForUi,
  dshSessionKey,
  dshStatusLabel,
  type DshSession,
} from "./dsh-sessions";

const session = (overrides: Partial<DshSession> = {}): DshSession => ({
  id: "session-1",
  pluginInstanceId: "plugin-1",
  bridgeInstanceId: "bridge-1",
  status: "waitingForInput",
  title: "DeepSeek Harness session",
  cwd: "C:/work",
  startedAt: 1,
  updatedAt: 2,
  activities: [],
  outputs: [],
  question: null,
  permission: null,
  plan: null,
  ...overrides,
});

describe("dsh session presentation", () => {
  it("maps DSH questions onto the shared question flow", () => {
    const request = dshQuestionForUi({
      id: "request-1",
      bridgeInstanceId: "bridge-1",
      pluginInstanceId: "plugin-1",
      sessionId: "session-1",
      capturedAt: 2,
      questions: [
        {
          id: "question-1",
          header: "Mode",
          question: "Select mode",
          detail: "This affects the next tool call.",
          options: [{ label: "Safe", description: null }],
          multiSelect: false,
          intent: null,
        },
      ],
    });

    expect(request).toMatchObject({
      id: "request-1",
      questions: [
        {
          question: "Select mode\n\nThis affects the next tool call.",
          allowOther: true,
          allowChat: false,
        },
      ],
    });
  });

  it("prioritizes plan review over permission and question", () => {
    const pending = dshPendingReview([
      session({
        question: {
          id: "question-1",
          bridgeInstanceId: "bridge-1",
          pluginInstanceId: "plugin-1",
          sessionId: "session-1",
          capturedAt: 2,
          questions: [],
        },
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
        plan: {
          id: "plan-1",
          bridgeInstanceId: "bridge-1",
          pluginInstanceId: "plugin-1",
          sessionId: "session-1",
          toolName: "exit_plan_mode",
          plan: "# Plan",
          approveLabel: "Approve",
          declineLabel: "Continue planning",
          cwd: "C:/work",
          capturedAt: 2,
        },
      }),
    ]);

    expect(pending?.view).toBe("plan");
    expect(pending?.request.id).toBe("plan-1");
  });

  it("exposes stable keys, labels, and one-shot capabilities", () => {
    expect(dshSessionKey(session())).toBe("dsh:plugin-1:session-1");
    expect(dshStatusLabel("waitingForApproval")).toBe("等待审批");
    expect(DSH_REVIEW_CAPABILITIES).toMatchObject({
      canAllowOnce: true,
      canAlwaysAllow: false,
      canDeny: true,
      canKeepPlanning: true,
    });
  });
});
