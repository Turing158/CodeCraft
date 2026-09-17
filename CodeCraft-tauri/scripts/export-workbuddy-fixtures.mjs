// Export only Hook documents and permission metadata, never model prompts,
// debug logs, credentials, raw stdout/stderr or a user's settings.
import fs from "node:fs/promises";
import path from "node:path";
import crypto from "node:crypto";
import { fileURLToPath } from "node:url";

const project = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const base = path.join(project, ".workbuddy-verification");
const output = path.join(project, "protocol/workbuddy/5.5.3/fixtures/runtime");
const digest = (text) => crypto.createHash("sha256").update(text).digest("hex");
await fs.mkdir(output, { recursive: true });
const selected = process.argv.slice(2);
if (!selected.length) throw new Error("Pass isolated capture directory names to export");

for (const name of selected) {
  if (!/^[a-z-]+-[A-Za-z0-9]+$/.test(name)) throw new Error("Invalid capture directory name");
  const directory = path.join(base, name);
  const bytes = await fs.readFile(path.join(directory, "capture.json"));
  const capture = JSON.parse(bytes);
  const policy = JSON.parse(await fs.readFile(path.join(directory, "policy.json"), "utf8"));
  if (policy.tool === "ExitPlanMode" && Object.hasOwn(policy.args, "plan")) {
    throw new Error("Invalid ExitPlanMode plan argument must not become evidence");
  }
  const ids = new Map();
  const sessionId = (id) => {
    if (!ids.has(id)) ids.set(id, `native-session-${ids.size + 1}`);
    return ids.get(id);
  };
  const paths = new Map();
  function sanitize(value, key = "", depth = 0) {
    if (depth > 12) return "[DEPTH LIMIT]";
    if (/token|password|secret|authorization|cookie|headers|environment/i.test(key)) return "[REDACTED]";
    if (typeof value === "string") {
      if (["session_id", "sessionId"].includes(key)) return sessionId(value);
      if (key === "transcript_path") {
        if (!paths.has(value)) paths.set(value, `<TRANSCRIPT_${paths.size + 1}>`);
        return paths.get(value);
      }
      if (key === "cwd") return "<WORKSPACE>";
      if (["file_path", "path"].includes(key)) return `<WORKSPACE>/${path.win32.basename(value)}`;
      if (["generation_id", "requestId"].includes(key)) return "<RUNTIME_ID>";
      if (key === "prompt" && value !== "Run the single isolated CodeCraft protocol fixture tool call, then stop.") {
        return `[REDACTED PROMPT sha256=${digest(value)}]`;
      }
      let text = value.replaceAll(directory, "<CAPTURE_ROOT>")
        .replaceAll(directory.replaceAll("\\", "/"), "<CAPTURE_ROOT>")
        .replace(/(?:Bearer|Basic)\s+\S+/gi, "[REDACTED AUTH]")
        .replace(/https?:\/\/[^\s"']+/g, "<URL>");
      text = text.replace(/[A-Za-z]:[\\/][^\s"'<>]+/g, "<LOCAL_PATH>");
      if (text.length > 8192) text = `${text.slice(0, 7900)} [TRUNCATED sha256=${digest(value)} bytes=${Buffer.byteLength(value)}]`;
      return text;
    }
    if (Array.isArray(value)) return value.slice(0, 64).map((item) => sanitize(item, "", depth + 1));
    if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).slice(0, 128)
      .map(([key, item]) => [key, sanitize(item, key, depth + 1)]));
    return value;
  }
  const hookNames = (await fs.readdir(directory)).filter((file) => file.endsWith(".hook.json")).sort();
  const hooks = [];
  for (const file of hookNames) {
    const hookBytes = await fs.readFile(path.join(directory, file));
    const hook = JSON.parse(hookBytes);
    hooks.push({ input: sanitize(hook.input), response: sanitize(hook.response), originalSha256: digest(hookBytes) });
  }
  const nativeRequests = (capture.nativeRequests ?? []).map((request) => ({
    method: request.method,
    sessionId: sessionId(request.params.sessionId),
    toolName: request.params.toolCall?._meta?.["codebuddy.ai/toolName"] ?? null,
    input: sanitize(request.params.toolCall?.rawInput),
    options: sanitize(request.params.options),
    verifierResponse: "reject_once_or_cancelled; no approval or answer supplied",
  }));
  const transport = capture.transport ?? "print";
  const toolResults = capture.stdout.split("\n").flatMap((line) => {
    try { return JSON.parse(line).message?.content?.filter((item) => item.type === "tool_result") ?? []; }
    catch { return []; }
  }).map((result) => ({
    toolUseId: result.tool_use_id,
    isError: result.is_error,
    rawResponse: sanitize(result._meta?.rawResponse ?? null),
    text: sanitize((result.content ?? []).map((part) => part.text ?? "").join("\n")).slice(0, 2048),
  }));
  const fixture = {
    schemaVersion: 1,
    evidenceKind: "real-bundled-cli-with-deterministic-local-model",
    scenario: capture.scenario,
    transport,
    desktopUiValidated: false,
    protocolFrozen: false,
    provenance: {
      ...(capture.provenance ?? { hashAtCapture: null, note: "Legacy capture before provenance hashing" }),
      cliVersions: [...new Set(hooks.map((hook) => hook.input.version).filter(Boolean))],
      entry: "<WORKBUDDY_INSTALL>/resources/app.asar.unpacked/cli/bin/codebuddy",
      originalCaptureSha256: digest(bytes),
      isolatedSettings: true,
      localModelOnly: true,
    },
    policy: sanitize(policy),
    result: {
      exitCode: capture.exitCode,
      promptCompleted: capture.completed ?? null,
      verifierTimedOut: capture.timedOut,
      toolCallSentByLocalModel: capture.toolSent,
      workspaceFiles: capture.workspaceFiles,
      nativePermissionRequests: nativeRequests.length,
      hookEventOrder: hooks.map((hook) => hook.input.hook_event_name),
    },
    nativeRequests,
    toolResults,
    hooks,
  };
  const filename = `${capture.scenario}.${transport}.runtime.json`;
  await fs.writeFile(path.join(output, filename), JSON.stringify(fixture, null, 2) + "\n");
  console.log(`${filename}: ${hooks.length} real Hook documents, ${nativeRequests.length} native permission requests`);
}
