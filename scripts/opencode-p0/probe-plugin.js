import { appendFile } from "node:fs/promises";

const logPath = process.env.CODECRAFT_P0_LOG;
const mode = process.env.CODECRAFT_P0_BEFORE_MODE ?? "observe";
const maxText = 512;
let writeQueue = Promise.resolve();

function clip(value) {
  if (typeof value !== "string") return value;
  return value.length <= maxText ? value : `${value.slice(0, maxText)}...[truncated]`;
}

function isSensitiveKey(key) {
  return /(authorization|cookie|secret|token|password|api[-_]?key|credential|environment|env|header|command|args|output|diff|content|text|prompt|message)/i.test(key);
}

function safeValue(value, key = "", depth = 0) {
  if (isSensitiveKey(key)) return "[redacted]";
  if (value === null || typeof value === "boolean" || typeof value === "number" || typeof value === "string") {
    return clip(value);
  }
  if (depth >= 2) return Array.isArray(value) ? `[array:${value.length}]` : "[object]";
  if (Array.isArray(value)) return value.slice(0, 16).map((item) => safeValue(item, key, depth + 1));
  if (typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value)
        .slice(0, 32)
        .map(([entryKey, entryValue]) => [entryKey, safeValue(entryValue, entryKey, depth + 1)]),
    );
  }
  return String(value);
}

function eventRecord(event) {
  const properties = event?.properties;
  return {
    id: typeof event?.id === "string" ? event.id : undefined,
    type: typeof event?.type === "string" ? event.type : typeof event,
    propertyKeys: properties && typeof properties === "object" ? Object.keys(properties).slice(0, 64) : [],
    properties: safeValue(properties),
  };
}

async function record(record) {
  if (!logPath) return;
  const line = `${JSON.stringify({ at: new Date().toISOString(), ...record })}\n`;
  writeQueue = writeQueue.then(() => appendFile(logPath, line, "utf8"));
  await writeQueue;
}

function methodShape(value) {
  if (!value || (typeof value !== "object" && typeof value !== "function")) return [];
  const own = Object.keys(value);
  const prototype = Object.getOwnPropertyNames(Object.getPrototypeOf(value) ?? {}).filter((key) => key !== "constructor");
  return [...new Set([...own, ...prototype])].sort();
}

export default async function probePlugin(input) {
  const client = input?.client;
  const shape = {
    client: methodShape(client),
    nested: Object.fromEntries(
      ["question", "permission", "session", "event", "global", "project", "config", "tool"].map((key) => [
        key,
        methodShape(client?.[key]),
      ]),
    ),
    contextKeys: Object.keys(input ?? {}).sort(),
    directory: typeof input?.directory === "string" ? input.directory : undefined,
    worktree: typeof input?.worktree === "string" ? input.worktree : undefined,
    serverUrl: input?.serverUrl instanceof URL ? input.serverUrl.protocol : undefined,
  };
  await record({ kind: "plugin.loaded", shape });

  await record({
    kind: "client.pending-list-shape",
    pendingLists: Object.fromEntries(
      ["question", "permission"].map((name) => [name, { list: typeof client?.[name]?.list === "function" }]),
    ),
  });

  return {
    event({ event }) {
      record({ kind: "event", event: eventRecord(event) });
    },
    "tool.execute.before": async (inputValue, output) => {
      await record({
        kind: "tool.before.enter",
        tool: inputValue?.tool,
        sessionID: inputValue?.sessionID,
        callID: inputValue?.callID,
        argKeys: output?.args && typeof output.args === "object" ? Object.keys(output.args).slice(0, 64) : [],
        args: safeValue(output?.args),
      });
      if (mode === "delay") await new Promise((resolve) => setTimeout(resolve, 250));
      if (mode === "reject") throw new Error("CodeCraft P0 probe rejection");
      await record({ kind: "tool.before.exit", tool: inputValue?.tool, callID: inputValue?.callID, mode });
    },
    "tool.execute.after": async (inputValue, output) => {
      await record({
        kind: "tool.after",
        tool: inputValue?.tool,
        sessionID: inputValue?.sessionID,
        callID: inputValue?.callID,
        outputKeys: output && typeof output === "object" ? Object.keys(output).slice(0, 64) : [],
        outputLengths: Object.fromEntries(
          ["title", "output"].map((key) => [key, typeof output?.[key] === "string" ? output[key].length : undefined]),
        ),
      });
    },
  };
}
