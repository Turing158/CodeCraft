import { describe, expect, it, vi } from "vitest";

// @ts-expect-error The bundled DSH plugin is runtime JavaScript without a TS declaration.
import { apply, dshQuestionPayload, installUserQuestionBridge } from "../src-tauri/assets/dsh/codecraft/index.mjs";

const agent = {
  id: "session-1",
  session: { header: { cwd: "C:/work" } },
};

describe("DSH user-question provider bridge", () => {
  it("preserves regular and multi-select question payloads", () => {
    expect(
      dshQuestionPayload({
        agent,
        questions: [
          {
            id: "mode",
            header: "选择测试模式",
            question: "请选择测试模式",
            options: [
              { label: "快速", description: "只运行快速检查" },
              { label: "完整", description: "运行完整测试" },
            ],
            multiSelect: false,
          },
          {
            id: "fruit",
            question: "下列多选你喜欢什么",
            options: ["苹果", "香蕉", "梨子", "樱桃", "榴莲"].map((label) => ({ label })),
            multiSelect: true,
          },
        ],
      }),
    ).toEqual({
      requestKind: "question",
      questions: [
        {
          id: "mode",
          header: "选择测试模式",
          question: "请选择测试模式",
          detail: null,
          options: [
            { label: "快速", description: "只运行快速检查" },
            { label: "完整", description: "运行完整测试" },
          ],
          multiSelect: false,
          intent: null,
        },
        {
          id: "fruit",
          header: null,
          question: "下列多选你喜欢什么",
          detail: null,
          options: ["苹果", "香蕉", "梨子", "樱桃", "榴莲"].map((label) => ({
            label,
            description: null,
          })),
          multiSelect: true,
          intent: null,
        },
      ],
    });
  });

  it("routes plan-review requests through the plan channel", () => {
    expect(
      dshQuestionPayload({
        questions: [
          {
            id: "plan-review",
            question: "Approve this plan?",
            detail: "# Read-only inspection\n\n1. Read README.md",
            options: [
              { label: "Approve" },
              { label: "Continue planning" },
            ],
            intent: { kind: "plan-review", approve: "Approve" },
          },
        ],
      }),
    ).toMatchObject({
      requestKind: "plan",
      questions: [
        {
          id: "plan-review",
          detail: "# Read-only inspection\n\n1. Read README.md",
          intent: { kind: "plan-review", approve: "Approve" },
        },
      ],
    });
  });

  it("uses the bridge provider and restores the original provider on disposal", async () => {
    const originalProvider = {
      ask: vi.fn(async (..._args: unknown[]) => ({ answers: [{ id: "fallback", selected: [] }] })),
    };
    const userQuestions = { provider: originalProvider };
    const exchange = vi.fn(async () => ({
      available: true,
      response: {
        decision: "answer",
        answers: [{ id: "mode", selected: ["快速"] }],
      },
    }));

    const dispose = installUserQuestionBridge({ userQuestions }, exchange);
    const answer = await userQuestions.provider.ask({
      agent,
      questions: [{
        id: "mode",
        question: "请选择测试模式",
        options: [{ label: "快速" }, { label: "完整" }],
      }],
      signal: new AbortController().signal,
    });

    expect(answer).toEqual({ answers: [{ id: "mode", selected: ["快速"] }] });
    expect(exchange).toHaveBeenCalledWith(
      "request",
      expect.objectContaining({ requestKind: "question" }),
      expect.objectContaining({ requestId: expect.any(String), sessionId: "session-1", workspace: "C:/work" }),
      true,
      expect.any(AbortSignal),
    );
    expect(originalProvider.ask).not.toHaveBeenCalled();

    dispose();
    expect(userQuestions.provider).toBe(originalProvider);
  });

  it("falls back to the existing provider when the bridge is unavailable", async () => {
    const originalProvider = {
      ask: vi.fn(async (..._args: unknown[]) => ({ answers: [{ id: "mode", selected: ["完整"] }] })),
    };
    const userQuestions = { provider: originalProvider };
    const dispose = installUserQuestionBridge(
      { userQuestions },
      vi.fn(async () => ({ available: false })),
    );

    await expect(
      userQuestions.provider.ask({
        agent,
        questions: [{ id: "mode", question: "请选择测试模式" }],
      }),
    ).resolves.toEqual({ answers: [{ id: "mode", selected: ["完整"] }] });
    expect(originalProvider.ask).toHaveBeenCalledOnce();

    dispose();
  });

  it("does not require a provider before apiProxy has finished composing", async () => {
    const context = {
      on: vi.fn(),
      effect: vi.fn(),
      inject: vi.fn(),
    };
    apply(context);

    // apiProxy is the owner of the Web provider, so the callback must not run
    // until both it and the userQuestions seam are available.
    expect(context.inject).toHaveBeenCalledWith(
      ["apiProxy", "userQuestions"],
      expect.any(Function),
    );
  });
});
