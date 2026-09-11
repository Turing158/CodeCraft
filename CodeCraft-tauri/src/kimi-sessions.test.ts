import { describe, expect, it } from "vitest";
import { kimiInteractionFor, kimiReviewFor, type KimiInteraction, type KimiSession, type KimiSnapshot } from "./kimi-sessions";
import { isGeminiSession, isKimiSession, unifiedSessionKey, unifiedSessionSource } from "./unified-sessions";
import { canAct, mergeSnapshot } from "../web/lan/snapshot";

const observation: KimiInteraction = {
  observationId: "observation", interactionKey: "session:call", kind: "shell",
  status: "observed", title: "Kimi", detail: "echo test", toolName: "Shell",
  toolCallId: "call", toolInput: null, questions: [], planFilename: null,
  plan: null, planReadError: null, capturedAt: 2, truncated: false, navigationAvailable: false,
};
const session: KimiSession = {
  id: "session", kimiSessionId: "native", integrationSessionKey: "session",
  hookInstallId: null, clientType: "kimi_code_cli", title: "Kimi", cwd: "C:/work",
  status: "toolRunning", startedAt: 1, updatedAt: 2, endedAt: null,
  activities: [], outputs: [], pendingInteractions: [observation], terminalBinding: null,
  integrationStatus: "running",
};
const snapshot: KimiSnapshot = {
  connected: true, integrationError: null, version: 1, sessions: [session], interactions: [observation],
  navigationCapability: "unsupported", fallbackAction: "openKimiOnHost",
  capabilities: { canObserve: true, canApproveTools: false, canAnswerQuestions: false,
    canApprovePlans: false, canStreamOutput: false, readOnly: true, reason: "read-only" },
};

describe("Kimi read-only integration", () => {
  it("keeps structurally similar Kimi and Gemini sessions distinct", () => {
    expect(isKimiSession(session)).toBe(true);
    expect(isGeminiSession(session)).toBe(false);
    expect(unifiedSessionSource(session)).toBe("kimi");
    expect(unifiedSessionKey(session)).toBe("kimi:session");
  });

  it.each(["shell", "askUser", "exitPlanMode"] as const)("cannot submit %s observations even when LAN approvals are enabled", (kind) => {
    const merged = mergeSnapshot({ allowApprovals: true, kimi: {
      ...snapshot, sessions: [{ ...session, pendingInteractions: [{ ...observation, kind }] }],
    }, integrations: [{ id: "kimiCode", hookInstalled: true, agentInstalled: true }] });
    const pending = merged.entries[0].pending;
    expect(merged.entries[0].source).toBe("kimi");
    expect(merged.integrations[0].name).toBe("Kimi Code");
    expect(pending?.kind).toBe(kind === "askUser" ? "question" : kind === "exitPlanMode" ? "plan" : "permission");
    expect(pending?.kimi?.sessionId).toBe("session");
    expect(canAct(pending, true)).toBe(false);
  });

  it("expires completed turns and stale observations", () => {
    for (const status of ["idle", "stopped"] as const) {
      const ended = { ...session, status };
      expect(kimiInteractionFor(ended)).toBeUndefined();
      expect(mergeSnapshot({ kimi: { ...snapshot, sessions: [ended] } }).entries[0].pending).toBeNull();
    }
    expect(kimiInteractionFor({ ...session, pendingInteractions: [{ ...observation, status: "stale" }] })).toBeUndefined();
  });

  it("maps tool observations to the permission view without approval capabilities", () => {
    expect(kimiReviewFor(session)).toEqual({ kind: "permission", request: {
      id: observation.interactionKey, toolName: "Shell", summary: "echo test",
      cwd: "C:/work", canAlwaysAllow: false, capturedAt: 2,
    } });
  });

  it("preserves all questions and options while disabling answers", () => {
    const review = kimiReviewFor({ ...session, pendingInteractions: [{
      ...observation, kind: "askUser", questions: [
        { header: "Files", question: "Which files?", multi_select: true, options: ["a", { label: "b", description: "Second file" }] },
        "Which format?",
      ],
    }] });
    expect(review?.kind).toBe("question");
    if (review?.kind !== "question") throw new Error("Missing question review");
    expect(review.request.questions).toHaveLength(2);
    expect(review.request.questions[0]).toMatchObject({ multiSelect: true, readOnly: true, allowOther: false, allowChat: false });
    expect(review.request.questions[0].answerMode).toBe("Kimi Code");
    expect(review.request.questions[0].options.map((option) => option.label)).toEqual(["a", "b"]);
    expect(review.request.questions[1].question).toBe("Which format?");
  });

  it("keeps plan file errors visible in the plan view", () => {
    const review = kimiReviewFor({ ...session, pendingInteractions: [{ ...observation,
      kind: "exitPlanMode", planFilename: "plan.md", planReadError: "File missing",
    }] });
    expect(review?.kind).toBe("plan");
    if (review?.kind !== "plan") throw new Error("Missing plan review");
    expect(review.request.plan).toContain("plan.md");
    expect(review.request.plan).toContain("File missing");
  });

  it("does not open review views for notifications or completed observations", () => {
    expect(kimiReviewFor({ ...session, pendingInteractions: [{ ...observation, kind: "notification" }] })).toBeUndefined();
    expect(kimiReviewFor({ ...session, pendingInteractions: [{ ...observation, resolved: true }] })).toBeUndefined();
    for (const status of ["toolCompleted", "sessionEnded", "stale"] as const) {
      expect(kimiReviewFor({ ...session, pendingInteractions: [{ ...observation, status }] })).toBeUndefined();
    }
  });

  it("keeps the review id stable when a permission notification updates the same call", () => {
    const initial = kimiReviewFor(session);
    const updated = kimiReviewFor({ ...session, pendingInteractions: [{ ...observation, observationId: "permission-notification", detail: "Updated command" }] });
    expect(updated?.request.id).toBe(initial?.request.id);
  });
});
