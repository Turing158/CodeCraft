import {
  mkdir,
  readFile,
  readdir,
  rename,
  unlink,
  writeFile,
} from "node:fs/promises";
import { createHash, randomUUID } from "node:crypto";
import {
  CONFIG_DIR_NAME,
  VERSION,
  getAgentDir,
} from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";

const SCHEMA_VERSION = "1.0";
const PROTOCOL_VERSION = "1.0";
const EXTENSION_VERSION = "0.2.1";
const MAX_ENVELOPE_BYTES = 256 * 1024;
const HEARTBEAT_INTERVAL_MS = 5_000;
const NO_EXPIRY_MS = 0;
const MAX_DISPLAY_TEXT = 64 * 1024;
const root = process.env.LOCALAPPDATA
  ? `${process.env.LOCALAPPDATA}/CodeCraft/pi-hook`
  : `${process.env.USERPROFILE ?? process.env.HOME ?? "."}/.codecraft/pi-hook`;
const instanceId = randomUUID();
const streamEpoch = randomUUID();
let sequence = 0;
let writeQueue = Promise.resolve();
const sessionRules = new Set<string>();
const outputIds = new Map<string, string>();
const CONTROL_TOOLS = new Set(["codecraft_questionnaire"]);

const QuestionOptionSchema = Type.Object({
  value: Type.String({ description: "Value returned for this option" }),
  label: Type.String({ description: "Label shown to the user" }),
  description: Type.Optional(Type.String()),
});

const QuestionSchema = Type.Object({
  id: Type.String({ description: "Stable question identifier" }),
  label: Type.Optional(Type.String({ description: "Short section label" })),
  prompt: Type.String({ description: "Question shown to the user" }),
  options: Type.Array(QuestionOptionSchema),
  multiSelect: Type.Optional(Type.Boolean()),
  allowOther: Type.Optional(Type.Boolean()),
  isSecret: Type.Optional(Type.Boolean()),
});

const QuestionnaireSchema = Type.Object({
  questions: Type.Array(QuestionSchema),
});

const encodePath = (value: string) => value.replaceAll("\\", "/");
const installId = process.env.CODECRAFT_PI_INSTALL_ID ?? "unmanaged";

const sessionIdFor = (ctx: any): string => {
  const value = ctx?.sessionManager?.getSessionId?.();
  return typeof value === "string" && value.length > 0
    ? value
    : `transient-${instanceId}`;
};

const digest = (value: unknown) =>
  createHash("sha256").update(JSON.stringify(value)).digest("hex");

const safe = (value: unknown, key = "", depth = 0): unknown => {
  const normalized = key.replaceAll("-", "").replaceAll("_", "").toLowerCase();
  if (normalized === "displaytext" && typeof value === "string") {
    return value.length > MAX_DISPLAY_TEXT
      ? `${value.slice(0, MAX_DISPLAY_TEXT)}...[truncated]`
      : value;
  }
  if (["authorization", "cookie", "secret", "token", "password", "apikey", "credential", "environment", "env", "header", "headers", "command", "args", "output", "diff", "content", "text", "prompt", "message"].includes(normalized)) return "[redacted]";
  if (value === null || typeof value === "boolean" || typeof value === "number") return value;
  if (typeof value === "string") return value.length > 512 ? `${value.slice(0, 512)}...[truncated]` : value;
  if (depth >= 6) return Array.isArray(value) ? `[array:${value.length}]` : "[object]";
  if (Array.isArray(value)) return value.slice(0, 32).map((item) => safe(item, key, depth + 1));
  if (typeof value === "object") return Object.fromEntries(Object.entries(value).slice(0, 64).map(([entryKey, entryValue]) => [entryKey, safe(entryValue, entryKey, depth + 1)]));
  return String(value);
};

const truncate = (value: string, length = 512) =>
  value.length > length ? `${value.slice(0, length)}...[truncated]` : value;

const messageText = (message: any): string => {
  if (message?.role !== "assistant") return "";
  const content = message?.content;
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .filter((part) => part?.type === "text" && typeof part?.text === "string")
    .map((part) => part.text)
    .join("\n")
    .trim();
};

const toolInputSummary = (input: unknown): string => {
  if (input === null || input === undefined) return "无参数";
  if (typeof input === "string") return truncate(input);
  if (typeof input !== "object") return String(input);
  const record = input as Record<string, unknown>;
  const command = Object.entries(record).find(([key, value]) => {
    const normalized = key
      .replaceAll("-", "")
      .replaceAll("_", "")
      .toLowerCase();
    return normalized === "command" && typeof value === "string" && value.trim();
  })?.[1];
  if (typeof command === "string") return truncate(command.trim());
  const summary: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(record).slice(0, 32)) {
    const normalized = key.replaceAll("-", "").replaceAll("_", "").toLowerCase();
    if (["authorization", "cookie", "secret", "token", "password", "apikey", "credential", "environment", "env", "header", "headers", "command", "args", "output", "diff", "content", "text", "prompt", "message"].includes(normalized)) {
      summary[key] = "[redacted]";
    } else if (typeof value === "string") {
      summary[key] = truncate(value);
    } else if (value === null || typeof value === "boolean" || typeof value === "number") {
      summary[key] = value;
    } else if (Array.isArray(value)) {
      summary[key] = `[array:${value.length}]`;
    } else {
      summary[key] = "[object]";
    }
  }
  return JSON.stringify(summary);
};

const toolRisk = (toolName: string, input: unknown): "low" | "elevated" | "high" => {
  const normalized = toolName.toLowerCase();
  if (["read", "glob", "grep", "find", "ls"].includes(normalized)) return "low";
  const command =
    typeof input === "object" && input !== null && typeof (input as Record<string, unknown>).command === "string"
      ? String((input as Record<string, unknown>).command)
      : "";
  const commandText = command.toLowerCase();
  if (
    commandText.includes("rm -rf") ||
    commandText.includes("git reset --hard") ||
    commandText.includes("format ") ||
    commandText.includes("shutdown") ||
    commandText.includes("taskkill")
  ) return "high";
  if (["bash", "shell", "exec", "command", "powershell"].includes(normalized)) {
    return commandText && /^(pwd|cd |ls|dir|rg |grep |git status|git diff|git log|git show)/.test(commandText)
      ? "low"
      : "elevated";
  }
  return "elevated";
};

async function emit(
  messageType: string,
  payload: unknown,
  sessionId?: string,
  runId?: string,
  ttlMs = NO_EXPIRY_MS,
) {
  const now = Date.now();
  const envelope = {
    schemaVersion: SCHEMA_VERSION,
    protocolVersion: PROTOCOL_VERSION,
    messageType,
    messageId: randomUUID(),
    installId,
    extensionInstanceId: instanceId,
    endpointEpoch: `${instanceId}:generation:0`,
    streamId: sessionId ? `session:${sessionId}` : "control",
    streamEpoch,
    sessionId: sessionId ?? null,
    runId: runId ?? null,
    origin: "pi",
    eventType: messageType,
    sequence: ++sequence,
    cwd: process.cwd(),
    createdAt: now,
    expiresAt: ttlMs === NO_EXPIRY_MS ? NO_EXPIRY_MS : now + ttlMs,
    payload: safe(payload),
  };
  const bytes = Buffer.from(JSON.stringify(envelope));
  if (bytes.byteLength > MAX_ENVELOPE_BYTES) return;
  const directory = `${root}/inbox/${instanceId}`;
  const path = `${directory}/${envelope.messageId}.json`;
  const temporary = `${path}.${process.pid}.${randomUUID()}.tmp`;
  writeQueue = writeQueue.then(async () => {
    await mkdir(directory, { recursive: true });
    await writeFile(temporary, bytes, { flag: "wx" });
    await rename(temporary, path);
  });
  await writeQueue;
}

async function waitForDecision(
  requestId: string,
  deadline: number | undefined,
  signal: AbortSignal | undefined,
): Promise<Record<string, any>> {
  const directory = `${root}/outbox/${instanceId}`;
  while (deadline === undefined || Date.now() < deadline) {
    if (signal?.aborted) return { decision: "deny", reason: "PI 会话已取消" };
    try {
      const names = await readdir(directory);
      for (const name of names) {
        if (!name.endsWith(".json")) continue;
        const path = `${directory}/${name}`;
        let envelope: any;
        try {
          envelope = JSON.parse(await readFile(path, "utf8"));
        } catch {
          continue;
        }
        if (envelope?.messageType !== "decision" || envelope?.extensionInstanceId !== instanceId) continue;
        if (envelope?.payload?.requestId !== requestId) continue;
        await unlink(path).catch(() => undefined);
        return typeof envelope.payload?.decision === "string"
          ? envelope.payload
          : { decision: "deny", reason: "无效的 PI 审批决定" };
      }
    } catch {
      // CodeCraft may be starting up; keep polling until the request expires.
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  return { decision: "deny", reason: "CodeCraft 审批超时" };
}

async function requestToolApproval(event: any, ctx: any) {
  const toolName = typeof event?.toolName === "string" ? event.toolName : "tool";
  if (CONTROL_TOOLS.has(toolName.toLowerCase())) return undefined;
  const toolCallId = typeof event?.toolCallId === "string" ? event.toolCallId : randomUUID();
  const input = event?.input ?? event?.args ?? null;
  const inputDigest = digest(input);
  const sessionId = sessionIdFor(ctx);
  const ruleKey = `${sessionId}:${toolName.toLowerCase()}`;
  if (sessionRules.has(ruleKey)) return undefined;
  const requestId = randomUUID();
  const risk = toolRisk(toolName, input);
  const runId = ctx?.runId ?? undefined;
  const capturedAt = Date.now();
  await emit(
    "request",
    {
      requestId,
      toolCallId,
      toolName,
      requestKind: ["write", "edit"].includes(toolName.toLowerCase())
        ? "fileChange"
        : ["bash", "powershell", "shell", "exec", "command"].includes(toolName.toLowerCase())
          ? "commandRisk"
          : "toolCall",
      inputDigest,
      inputSummary: toolInputSummary(input),
      risk,
      capturedAt,
    },
    sessionId,
    runId,
  );
  const result = await waitForDecision(requestId, undefined, ctx?.signal);
  if (result.decision === "allowOnce") return undefined;
  if (result.decision === "allowSession") {
    sessionRules.add(ruleKey);
    return undefined;
  }
  await emit(
    "receipt",
    { requestId, outcome: "denied", reason: result.reason ?? "用户拒绝 PI 工具调用" },
    sessionId,
    runId,
  );
  return {
    block: true,
    reason: result.reason ?? "CodeCraft 拒绝了 PI 工具调用",
    terminate: true,
  };
}

const heartbeatPayload = () => ({
  extensionVersion: EXTENSION_VERSION,
  piVersion: VERSION,
  nodeVersion: process.version,
  configDirName: CONFIG_DIR_NAME,
  agentDir: encodePath(getAgentDir()),
  capabilities: {
    observation: true,
    toolGate: true,
    questionnaire: true,
  },
});

export default function codeCraftPiExtension(pi: any) {
  const runState = { id: undefined as string | undefined };
  void emit("heartbeat", heartbeatPayload());
  const heartbeatTimer = setInterval(() => {
    void emit("heartbeat", heartbeatPayload());
  }, HEARTBEAT_INTERVAL_MS);
  heartbeatTimer.unref?.();

  pi.registerTool({
    name: "codecraft_questionnaire",
    label: "CodeCraft Questionnaire",
    description:
      "Ask the user one or more non-secret questions in CodeCraft. Use this when requirements or preferences need clarification.",
    promptSnippet: "Ask the user structured, non-secret clarification questions in CodeCraft",
    promptGuidelines: [
      "Use codecraft_questionnaire for structured clarification questions when CodeCraft is connected.",
      "Never use codecraft_questionnaire for passwords, tokens, credentials, or other secrets.",
    ],
    parameters: QuestionnaireSchema,
    executionMode: "sequential",
    async execute(_toolCallId: string, params: any, signal: AbortSignal | undefined, _onUpdate: any, ctx: any) {
      if (!Array.isArray(params.questions) || params.questions.length === 0) {
        return {
          content: [{ type: "text", text: "No questions were provided." }],
          details: { cancelled: true, answers: [] },
        };
      }
      if (params.questions.some((question: any) => question.isSecret === true)) {
        return {
          content: [{ type: "text", text: "Secret questionnaire answers are not supported by CodeCraft." }],
          details: { cancelled: true, unsupported: "secret", answers: [] },
        };
      }

      const sessionId = sessionIdFor(ctx);
      const requestId = randomUUID();
      const capturedAt = Date.now();
      const questions = params.questions.map((question: any, index: number) => ({
        id: question.id || `question-${index + 1}`,
        header: question.label || `问题 ${index + 1}`,
        question: question.prompt,
        options: question.options,
        multiSelect: question.multiSelect === true,
        allowOther: question.allowOther !== false,
      }));
      await emit(
        "request",
        {
          requestId,
          requestKind: "questionnaire",
          questions,
          capturedAt,
        },
        sessionId,
        runState.id,
      );
      const result = await waitForDecision(requestId, undefined, signal);
      if (result.decision !== "answer" || !Array.isArray(result.answers)) {
        return {
          content: [{ type: "text", text: result.reason ?? "User cancelled the questionnaire." }],
          details: { cancelled: true, answers: [] },
        };
      }

      const answers = questions.map((question: any, index: number) => {
        const submitted = result.answers[index] ?? {};
        const labels = Array.isArray(submitted.selectedOptionLabels)
          ? submitted.selectedOptionLabels.filter((label: unknown) => typeof label === "string")
          : [];
        const selected = labels
          .map((label: string) => question.options.find((option: any) => option.label === label))
          .filter(Boolean);
        const extra = typeof submitted.extraText === "string" && submitted.extraText.trim()
          ? submitted.extraText.trim()
          : undefined;
        return {
          id: question.id,
          values: [...selected.map((option: any) => option.value), ...(extra ? [extra] : [])],
          labels: [...selected.map((option: any) => option.label), ...(extra ? [extra] : [])],
          wasCustom: Boolean(extra),
        };
      });
      return {
        content: [{ type: "text", text: JSON.stringify(answers) }],
        details: { cancelled: false, questions, answers },
      };
    },
  });

  for (const eventName of [
    "session_start",
    "session_info_changed",
    "session_shutdown",
    "before_agent_start",
    "agent_start",
    "agent_end",
    "agent_settled",
    "turn_start",
    "turn_end",
    "message_start",
    "message_update",
    "message_end",
    "tool_call",
    "tool_result",
    "tool_execution_start",
    "tool_execution_update",
    "tool_execution_end",
    "user_bash",
    "ui_prompt_start",
    "ui_prompt_end",
    "input",
  ]) {
    pi.on(eventName, async (event: any, ctx: any) => {
      const sessionId = sessionIdFor(ctx);
      if (eventName === "agent_start") runState.id = randomUUID();
      if (eventName === "tool_call") {
        const decision = await requestToolApproval(event, {
          ...ctx,
          runId: runState.id,
        });
        if (decision) return decision;
      }

      const payload: Record<string, unknown> = {
        eventType: eventName,
        event,
        mode: ctx?.mode,
        hasUI: ctx?.hasUI,
        sessionId,
        sessionFile: ctx?.sessionManager?.getSessionFile?.() ?? null,
        cwd: ctx?.cwd,
        signalPresent: Boolean(ctx?.signal),
        runId: runState.id ?? null,
      };
      if (eventName === "session_start" || eventName === "session_info_changed") {
        const name = pi.getSessionName?.();
        if (typeof name === "string" && name.trim()) payload.sessionTitle = truncate(name.trim(), 256);
      }
      if (eventName === "before_agent_start" && typeof event?.prompt === "string") {
        const title = event.prompt.trim().split(/\r?\n/, 1)[0];
        if (title) payload.sessionTitle = truncate(title, 96);
      }
      if (["message_start", "message_update", "message_end"].includes(eventName)) {
        const text = messageText(event?.message);
        if (event?.message?.role === "assistant") {
          if (eventName === "message_start" || !outputIds.has(sessionId)) {
            outputIds.set(sessionId, randomUUID());
          }
          if (text) {
            payload.outputId = outputIds.get(sessionId);
            payload.displayText = text;
            payload.outputFinal = eventName === "message_end";
          }
          if (eventName === "message_end") outputIds.delete(sessionId);
        }
      }

      await emit("event", payload, sessionId, runState.id);
      if (eventName === "session_shutdown" && ["quit", "reload"].includes(event?.reason)) {
        clearInterval(heartbeatTimer);
      }
    });
  }
}
