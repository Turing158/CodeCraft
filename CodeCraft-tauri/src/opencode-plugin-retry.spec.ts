import { describe, expect, it, vi } from "vitest";

// The installed OpenCode plugin is deliberately a standalone JavaScript file.
// @ts-expect-error The runtime plugin does not ship a TypeScript declaration.
import codecraftOpenCodePlugin, * as pluginModule from "../src-tauri/opencode-plugin/codecraft-opencode-plugin.js";

const { retrySessionAction } = codecraftOpenCodePlugin as typeof codecraftOpenCodePlugin & {
  retrySessionAction: (action: () => Promise<unknown>) => Promise<unknown>;
};
const { commandRisk, requiresUserDecision, shouldAutoApprove, toolRisk } = codecraftOpenCodePlugin as typeof codecraftOpenCodePlugin & {
  commandRisk: (command: string) => "low" | "elevated" | "high";
  requiresUserDecision: (tool: string) => boolean;
  shouldAutoApprove: (mode: string, tool: string, input: unknown) => boolean;
  toolRisk: (tool: string, input: unknown) => "low" | "elevated" | "high";
};

describe("OpenCode session action retry", () => {
  it("exposes only the OpenCode plugin entrypoint", () => {
    expect(Object.keys(pluginModule)).toEqual(["default"]);
    expect(typeof pluginModule.default).toBe("function");
  });

  it("retries a temporary session conflict and then succeeds", async () => {
    vi.useFakeTimers();
    try {
      const conflict = Object.assign(new Error("session is busy"), {
        status: 409,
      });
      const action = vi
        .fn<() => Promise<string>>()
        .mockRejectedValueOnce(conflict)
        .mockRejectedValueOnce(conflict)
        .mockResolvedValue("accepted");

      const result = retrySessionAction(action);
      await vi.advanceTimersByTimeAsync(100);
      await vi.advanceTimersByTimeAsync(250);

      await expect(result).resolves.toBe("accepted");
      expect(action).toHaveBeenCalledTimes(3);
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not retry permanent OpenCode errors", async () => {
    const action = vi.fn<() => Promise<void>>().mockRejectedValue(
      Object.assign(new Error("session not found"), { status: 404 }),
    );

    await expect(retrySessionAction(action)).rejects.toThrow(
      "session not found",
    );
    expect(action).toHaveBeenCalledTimes(1);
  });
});

describe("OpenCode approval policy", () => {
  it("keeps manual mode fully interactive", () => {
    expect(shouldAutoApprove("manual", "read", {})).toBe(false);
    expect(shouldAutoApprove("manual", "bash", { command: "git status" })).toBe(false);
  });

  it("only auto-approves low-risk work in risk mode", () => {
    expect(shouldAutoApprove("risk", "read", {})).toBe(true);
    expect(shouldAutoApprove("risk", "bash", { command: "git status" })).toBe(true);
    expect(shouldAutoApprove("risk", "edit", { path: "src/main.ts" })).toBe(false);
    expect(shouldAutoApprove("risk", "bash", { command: "npm install" })).toBe(false);
    expect(shouldAutoApprove("risk", "bash", { command: "git reset --hard HEAD~1" })).toBe(false);
  });

  it("auto-approves non-subjective tools in automatic mode", () => {
    expect(shouldAutoApprove("automatic", "edit", {})).toBe(true);
    expect(
      shouldAutoApprove("automatic", "bash", {
        command: "git reset --hard HEAD~1",
      }),
    ).toBe(true);
  });

  it("never auto-answers questions or plan confirmations", () => {
    for (const tool of [
      "question",
      "request_user_input",
      "AskUserQuestion",
      "plan_exit",
      "ExitPlanMode",
      "update_plan",
    ]) {
      expect(requiresUserDecision(tool)).toBe(true);
      expect(shouldAutoApprove("automatic", tool, {})).toBe(false);
    }
  });

  it("matches the shared command risk categories", () => {
    expect(commandRisk("git status")).toBe("low");
    expect(commandRisk("npm install")).toBe("elevated");
    expect(commandRisk("git reset --hard HEAD~1")).toBe("high");
    expect(toolRisk("write", { path: "README.md" })).toBe("elevated");
  });
});
