import { invoke } from "@tauri-apps/api/core";
import type { CodexSnapshot } from "./codex-sessions";

let statusEl: HTMLElement | undefined;
let latestSnapshot: CodexSnapshot = {
  connected: false,
  integrationError: null,
  version: 0,
  sessions: [],
  interactions: [],
};
let snapshotListener: ((snapshot: CodexSnapshot) => void) | undefined;

export function initCodexPanel(
  onSnapshot: (snapshot: CodexSnapshot) => void,
): void {
  statusEl = document.getElementById(
    "codex-connection-status",
  ) as HTMLElement | undefined;
  snapshotListener = onSnapshot;
}

export async function refreshCodexPanel(): Promise<void> {
  let snapshot: CodexSnapshot;
  try {
    snapshot = await invoke<CodexSnapshot>("list_codex_sessions");
  } catch (error) {
    snapshot = {
      ...latestSnapshot,
      connected: false,
      integrationError: `读取 Codex 状态失败：${String(error)}`,
      // A failed hook read must not retain the previous hook-derived content.
      sessions: [],
      interactions: [],
    };
  }
  latestSnapshot = snapshot;
  renderConnection(snapshot);
  snapshotListener?.(snapshot);
}

export function getLatestCodexSnapshot(): CodexSnapshot {
  return latestSnapshot;
}

function renderConnection(snapshot: CodexSnapshot): void {
  if (!statusEl) return;
  setStatusLabel(
    statusEl,
    snapshot.connected
      ? "Hook 正常"
      : snapshot.integrationError
        ? "Hook 异常"
        : "Hook 未安装",
  );
  statusEl.dataset.connected = String(snapshot.connected);
  statusEl.title = snapshot.connected
    ? "Codex Hook 正常"
    : snapshot.integrationError ?? "Codex Hook 未安装";
}

function setStatusLabel(element: HTMLElement, label: string): void {
  const labelElement = element.querySelector<HTMLElement>(
    ".session-source-card__status-label",
  );
  if (labelElement) labelElement.textContent = label;
}
