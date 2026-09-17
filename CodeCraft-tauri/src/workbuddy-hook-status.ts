import type { WorkBuddyHookState } from "./workbuddy-sessions";

/** One presentation for the Hook settings and session card. */
export const workBuddyHookPresentation = (hook: WorkBuddyHookState | undefined) => {
  if (!hook) return {
    label: "检查状态中", connected: false,
    detail: "正在读取 WorkBuddy Hook 状态，可点击刷新重新检查。",
  };
  if (hook.error) return {
    label: ["modified", "conflict", "incompatible"].includes(hook.state) ? "安装需处理" : "连接异常",
    connected: false, detail: hook.error,
  };
  if (!hook.filesInstalled) return {
    label: "未安装", connected: false,
    detail: "请先在 Hook 设置中安装 WorkBuddy Hook。",
  };
  if (!hook.enabled) return {
    label: "已禁用", connected: false,
    detail: "Hook 文件已安装，但插件已禁用。请在 WorkBuddy 中启用 CodeCraft 插件，再刷新状态。",
  };
  if (!hook.bridgeReady) return {
    label: "本地桥未就绪", connected: false,
    detail: "Hook 已安装并启用，本地连接尚未就绪。请刷新状态；若仍未恢复，请重启 CodeCraft。",
  };
  if (!hook.loaded) return {
    label: "等待加载", connected: false,
    detail: hook.registered
      ? "插件已登记，尚未收到 Hook 事件。请在 WorkBuddy 中发起一次会话，再刷新状态。"
      : "Hook 已安装并启用，等待 WorkBuddy 登记和加载。请启动或重启 WorkBuddy，发起一次会话后刷新状态。",
  };
  if (!hook.connected) return {
    label: "等待会话", connected: false,
    detail: "已收到 WorkBuddy Hook 事件，当前没有连接中的会话。请在 WorkBuddy 中发起或继续会话。",
  };
  return {
    label: "已连接 · 只读", connected: true,
    detail: "Hook 已安装、已启用，已收到当前 WorkBuddy 会话的事件。",
  };
};
