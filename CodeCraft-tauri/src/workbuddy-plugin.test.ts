import { describe, expect, it } from "vitest";
// @ts-expect-error The bundled plugin is runtime JavaScript without TS declarations.
import { validateConfig, validatePayload } from "../src-tauri/assets/workbuddy/codecraft/scripts/workbuddy-bridge.mjs";

const config = {
  protocol: "workbuddy-codebuddy-hooks", protocolVersion: 1,
  endpoint: "http://127.0.0.1:9999", token: "a".repeat(64),
  pluginInstanceId: "00000000-0000-0000-0000-000000000000", expiresAt: 600_001,
};

describe("WorkBuddy plugin trust boundary", () => {
  it("accepts one bounded JSON object only", () => {
    expect(validatePayload('{"hook_event_name":"Stop"}').hook_event_name).toBe("Stop");
    for (const input of ["null", "[]", '"text"', "{} {}", "{bad}", JSON.stringify({ x: "x".repeat(256 * 1024) })]) {
      expect(() => validatePayload(input)).toThrow();
    }
  });
  it("accepts only a bare numeric HTTP loopback endpoint", () => {
    expect(validateConfig(config, 1).hostname).toBe("127.0.0.1");
    expect(validateConfig({ ...config, endpoint: "http://[::1]:9999" }, 1).hostname).toBe("[::1]");
    for (const endpoint of ["http://localhost:9999", "https://127.0.0.1:9999", "http://192.168.1.1:9999", "http://127.0.0.1:9999/path", "http://user:pass@127.0.0.1:9999", "http://127.0.0.1:9999/?token=secret", "http://127.0.0.1:9999/#fragment"]) {
      expect(() => validateConfig({ ...config, endpoint }, 1)).toThrow();
    }
  });
  it("rejects expired, unbounded and malformed bridge credentials", () => {
    for (const change of [{ expiresAt: 0 }, { expiresAt: Number.MAX_SAFE_INTEGER }, { token: "secret" }, { pluginInstanceId: "" }, { protocolVersion: 2 }]) {
      expect(() => validateConfig({ ...config, ...change }, 1)).toThrow();
    }
  });
});
