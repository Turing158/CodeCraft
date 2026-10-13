import { afterEach, expect, it, vi } from "vitest";
import { submitCursor } from "./api";
afterEach(() => vi.unstubAllGlobals());
it("uses authenticated LAN submission with the shared CSRF header and exact request target", async () => {
  const fetch = vi.fn().mockResolvedValue(new Response(JSON.stringify({ accepted: true }), { status: 200 }));
  vi.stubGlobal("fetch", fetch);
  const body = { schemaVersion: 1, submission: { appEpoch: "epoch", requestId: "r", requestVersion: 3, argumentsDigest: "a", plan: null }, decision: { kind: "questionCancelled" } };
  await submitCursor("cancel", body);
  expect(fetch).toHaveBeenCalledWith("/api/cursor/cancel", expect.objectContaining({ credentials: "same-origin", method: "POST", headers: expect.objectContaining({ "X-CodeCraft-Lan": "1" }), body: JSON.stringify(body) }));
});
it("retains capability rejection so callers cannot mistake it for successful approval", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(JSON.stringify({ schemaVersion: 1, error: { code: "CAPABILITY_NOT_SUPPORTED", message: "Verification required" } }), { status: 409 })));
  await expect(submitCursor("plan", {})).rejects.toThrow("CAPABILITY_NOT_SUPPORTED: Verification required");
});
