const productsByHook = {
  claudeCode: "claude-code", codex: "codex", openCode: "opencode", mimo: "mimo",
  pi: "pi", deepSeekHarness: "dsh", zCode: "zcode", geminiCli: "gemini",
  kimiCode: "kimi", workBuddy: "workbuddy", trae: "trae", cursor: "cursor",
} as const;

export type SessionSourceProductId = typeof productsByHook[keyof typeof productsByHook];
type HookInstallation = { id: string; hookInstalled: boolean };

export function installedSessionProducts(statuses: ReadonlyArray<HookInstallation>): Set<SessionSourceProductId> {
  const installed = new Set<SessionSourceProductId>();
  for (const status of statuses) {
    if (!status.hookInstalled || !Object.prototype.hasOwnProperty.call(productsByHook, status.id)) continue;
    installed.add(productsByHook[status.id as keyof typeof productsByHook]);
  }
  return installed;
}

export const sessionProductVisible = (
  product: SessionSourceProductId,
  selected: SessionSourceProductId | "all",
  installed: ReadonlySet<SessionSourceProductId>,
): boolean => installed.has(product) && (selected === "all" || selected === product);

export const hookKnownUninstalled = (statuses: ReadonlyArray<HookInstallation>, id: string): boolean =>
  statuses.some(status => status.id === id && !status.hookInstalled);
