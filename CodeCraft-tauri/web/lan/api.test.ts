import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  submitDshPermission,
  submitDshPlan,
  submitDshQuestion,
  type DshTarget,
} from "./api";

const target: DshTarget = {
  bridgeInstanceId: "bridge-1",
  pluginInstanceId: "plugin-1",
  sessionId: "session-1",
};

describe("DSH LAN API", () => {
  const fetchMock = vi.fn<
    (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>
  >(async () =>
    new Response(JSON.stringify({ ok: true }), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    }),
  );

  beforeEach(() => {
    fetchMock.mockClear();
    vi.stubGlobal("fetch", fetchMock);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("submits permission, question, and plan identities to their DSH endpoints", async () => {
    await submitDshPermission(target, "permission-1", "allowOnce");
    await submitDshQuestion(target, "question-1", [
      {
        question: "Select mode",
        selectedOptionLabels: ["Safe"],
        extraText: "details",
      },
    ]);
    await submitDshPlan(target, "plan-1", false, "Keep the change smaller");

    expect(fetchMock).toHaveBeenNthCalledWith(
      1,
      "/api/dsh/permission",
      expect.objectContaining({ method: "POST" }),
    );
    expect(fetchMock).toHaveBeenNthCalledWith(
      2,
      "/api/dsh/question",
      expect.objectContaining({ method: "POST" }),
    );
    expect(fetchMock).toHaveBeenNthCalledWith(
      3,
      "/api/dsh/plan",
      expect.objectContaining({ method: "POST" }),
    );

    const permissionBody = JSON.parse(
      (fetchMock.mock.calls[0][1] as RequestInit).body as string,
    );
    const questionBody = JSON.parse(
      (fetchMock.mock.calls[1][1] as RequestInit).body as string,
    );
    const planBody = JSON.parse(
      (fetchMock.mock.calls[2][1] as RequestInit).body as string,
    );
    expect(permissionBody).toEqual({
      ...target,
      requestId: "permission-1",
      decision: "allowOnce",
    });
    expect(questionBody).toEqual({
      ...target,
      requestId: "question-1",
      answers: [
        {
          question: "Select mode",
          selectedOptionLabels: ["Safe"],
          extraText: "details",
        },
      ],
    });
    expect(planBody).toEqual({
      ...target,
      requestId: "plan-1",
      approved: false,
      feedback: "Keep the change smaller",
    });
  });
});
