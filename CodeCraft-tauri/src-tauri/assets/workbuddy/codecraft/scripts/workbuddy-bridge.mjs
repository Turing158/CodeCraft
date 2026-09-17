import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import crypto from "node:crypto";
import { pathToFileURL } from "node:url";

const MAX_INPUT_BYTES = 256 * 1024;
const MAX_CONFIG_BYTES = 16 * 1024;
const dataRoot = process.env.LOCALAPPDATA || process.env.APPDATA || os.homedir();
const configPath = process.env.CODECRAFT_WORKBUDDY_BRIDGE_CONFIG ||
  path.join(dataRoot, "CodeCraft", "workbuddy-bridge.json");

const safeReply = () => {
  // Empty JSON leaves the native WorkBuddy permission/UI behavior in control.
  process.stdout.write("{}\n");
};

export function validatePayload(input) {
  if (Buffer.byteLength(input) > MAX_INPUT_BYTES) throw new Error("input_too_large");
  const payload = JSON.parse(input);
  if (!payload || Array.isArray(payload) || typeof payload !== "object") {
    throw new Error("object_required");
  }
  return payload;
}

export function validateConfig(config, now = Date.now()) {
  if (config?.protocol !== "workbuddy-codebuddy-hooks" || config.protocolVersion !== 1) {
    throw new Error("unsupported_protocol");
  }
  const endpoint = new URL(config.endpoint);
  if (endpoint.protocol !== "http:" || !["127.0.0.1", "[::1]"].includes(endpoint.hostname) ||
      !endpoint.port || endpoint.username || endpoint.password || endpoint.pathname !== "/" ||
      endpoint.search || endpoint.hash) throw new Error("unsafe_endpoint");
  if (!/^[a-f0-9]{64}$/.test(config.token) ||
      !/^[a-f0-9-]{36}$/.test(config.pluginInstanceId)) throw new Error("invalid_identity");
  if (!Number.isSafeInteger(config.expiresAt) || config.expiresAt <= now ||
      config.expiresAt > now + 11 * 60_000) throw new Error("expired_token");
  return endpoint;
}

async function run() {
  // Bound incomplete stdin too; diagnostics never include input or credentials.
  const timer = setTimeout(() => process.stdin.destroy(new Error("input_timeout")), 5_000);
  timer.unref();
  try {
  const chunks = [];
  let total = 0;
  for await (const chunk of process.stdin) {
    total += chunk.length;
    if (total > MAX_INPUT_BYTES) throw new Error("input_too_large");
    chunks.push(chunk);
  }
  clearTimeout(timer);
  const input = Buffer.concat(chunks).toString("utf8").trim();
  if (!input) {
    return;
  }
  const payload = validatePayload(input);
  const file = await fs.open(configPath, "r");
  let config;
  try {
    if (!(await file.stat()).isFile()) throw new Error("invalid_config");
    const bytes = Buffer.alloc(MAX_CONFIG_BYTES + 1);
    const { bytesRead } = await file.read(bytes, 0, bytes.length, 0);
    if (bytesRead > MAX_CONFIG_BYTES) throw new Error("config_too_large");
    config = JSON.parse(bytes.subarray(0, bytesRead).toString("utf8"));
  } finally {
    await file.close();
  }
  const base = validateConfig(config);
  const nonce = crypto.randomUUID();
  const interactionEvent = payload.hook_event_name === "PreToolUse" ||
    payload.hook_event_name === "PermissionRequest" ||
    payload.hook_event_name === "Elicitation";
  const endpoint = new URL("/api/workbuddy/" +
    (interactionEvent ? "interactions" : "events"),
    base);
  const response = await fetch(endpoint, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "x-codecraft-workbuddy-token": config.token,
      "x-codecraft-workbuddy-nonce": nonce,
      "x-codecraft-workbuddy-plugin-id": config.pluginInstanceId,
      "x-codecraft-workbuddy-protocol": config.protocol,
      "x-codecraft-workbuddy-protocol-version": String(config.protocolVersion),
      "x-codecraft-workbuddy-sent-at": String(Date.now()),
      "x-codecraft-workbuddy-hook-pid": String(process.pid),
    },
    body: JSON.stringify(payload),
    signal: AbortSignal.timeout(interactionEvent ? 8_000 : 4_000),
    redirect: "error",
  });
  // This release never relays Hook decisions. Do not buffer an arbitrary body.
  await response.body?.cancel();
  if (!response.ok) throw new Error(`bridge_http_${response.status}`);
  } catch (error) {
  // Diagnostics stay on stderr. stdout is reserved for one WorkBuddy JSON.
  const code = error instanceof Error && /^[a-z_]+(?:_\d+)?$/.test(error.message)
    ? error.message : "bridge_unavailable_or_invalid_input";
  console.error(`CodeCraft WorkBuddy hook: ${code}`);
  } finally {
    clearTimeout(timer);
    safeReply();
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  await run();
}
