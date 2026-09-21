import { afterEach, expect, it, vi } from "vitest";
import { submitTrae } from "./api";
afterEach(() => vi.unstubAllGlobals());
it("carries LAN credentials, CSRF header and exact Trae decision DTO", async () => {
  const fetch = vi.fn().mockResolvedValue(new Response(JSON.stringify({ accepted: true }), { status: 200 })); vi.stubGlobal("fetch", fetch);
  const body = { schemaVersion: 1, decisionId: "d", target: { appEpoch: "e", sessionKey: "s", taskId: null, turnEpoch: 2, requestVersion: 1, requestId: "r" }, action: { kind: "permission", decision: "deny", message: null } };
  await submitTrae("permission", body);
  expect(fetch).toHaveBeenCalledWith("/api/trae/permission", expect.objectContaining({ credentials: "same-origin", method: "POST", headers: expect.objectContaining({ "X-CodeCraft-Lan": "1" }), body: JSON.stringify(body) }));
});
it("surfaces structured conflicts rather than losing the backend reason", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({ schemaVersion: 1, error: { code: "TASK_CHANGED", message: "Task ended" } }), { status: 409 })));
  await expect(submitTrae("plan", {})).rejects.toThrow("TASK_CHANGED: Task ended");
});
