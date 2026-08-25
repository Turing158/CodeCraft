import { invoke } from "@tauri-apps/api/core";

export interface OpenCodeSessionTarget {
  pluginInstanceId: string;
  sessionId: string;
}

export const openCodeSessionTransport = {
  switchAgent: (target: OpenCodeSessionTarget, agent: string): Promise<void> =>
    invoke("switch_opencode_agent", {
      pluginInstanceId: target.pluginInstanceId,
      sessionId: target.sessionId,
      agent,
    }),
  sendMessage: (
    target: OpenCodeSessionTarget,
    message: string,
  ): Promise<void> =>
    invoke("send_opencode_session_message", {
      pluginInstanceId: target.pluginInstanceId,
      sessionId: target.sessionId,
      message,
    }),
};
