import { describe, expect, it } from "vitest";
import { workBuddyHookPresentation } from "./workbuddy-hook-status";
import type { WorkBuddyHookState } from "./workbuddy-sessions";

const pending: WorkBuddyHookState = {
  state: "syncedRestartRequired", filesInstalled: true, enabled: true,
  registered: false, loaded: false, connected: false, bridgeReady: true, error: null,
};

describe("WorkBuddy installation and runtime presentation", () => {
  it("keeps installation visible while explaining the missing load step", () => {
    const view = workBuddyHookPresentation(pending);
    expect(view.label).toBe("等待加载");
    expect(view.detail).toContain("已安装并启用");
    expect(view.detail).toContain("登记和加载");
    expect(view.connected).toBe(false);
    expect(workBuddyHookPresentation({ ...pending, registered: true }).detail).toContain("插件已登记");
  });
  it("does not infer connection from installation, registration, or an available bridge", () => {
    expect(workBuddyHookPresentation({ ...pending, state: "installed", registered: true }).connected).toBe(false);
    expect(workBuddyHookPresentation({ ...pending, loaded: true }).label).toBe("等待会话");
    expect(workBuddyHookPresentation({ ...pending, loaded: true, connected: true }).connected).toBe(true);
  });
  it("distinguishes disabled, absent, and failed installations with recovery guidance", () => {
    expect(workBuddyHookPresentation({ ...pending, state: "disabled", enabled: false }).label).toBe("已禁用");
    expect(workBuddyHookPresentation({ ...pending, filesInstalled: false }).label).toBe("未安装");
    expect(workBuddyHookPresentation({ ...pending, bridgeReady: false }).detail).toContain("重启 CodeCraft");
    expect(workBuddyHookPresentation({ ...pending, state: "modified", error: "插件文件已被修改" }).label).toBe("安装需处理");
    expect(workBuddyHookPresentation({ ...pending, error: "连接中断" }).detail).toBe("连接中断");
    expect(workBuddyHookPresentation({ ...pending, loaded: true, connected: true, error: "连接中断" }).connected).toBe(false);
    expect(workBuddyHookPresentation({ ...pending, loaded: true, connected: true }).label).toBe("已连接 · 只读");
  });
});
