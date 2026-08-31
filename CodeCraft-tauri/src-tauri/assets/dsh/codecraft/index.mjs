import { createHash, randomUUID } from "node:crypto";
import { readFile } from "node:fs/promises";
import net from "node:net";
import os from "node:os";
import path from "node:path";

const PROTOCOL = "codecraft-dsh-bridge";
const PROTOCOL_VERSION = 1;
const PLUGIN_VERSION = "0.1.2";
const HEARTBEAT_INTERVAL_MS = 5_000;
const CONNECT_TIMEOUT_MS = 1_500;
const DECISION_TIMEOUT_MS = 5 * 60_000;
const MAX_LINE_BYTES = 256 * 1024;
const pluginInstanceId = randomUUID();
const toolCalls = new Map();

const packageVersion = async (candidate) => {
  try {
    const manifest = JSON.parse(await readFile(candidate, "utf8"));
    return manifest?.name === "@deepseek-ai/dsh" && typeof manifest?.version === "string"
      ? manifest.version
      : null;
  } catch {
    return null;
  }
};

const detectDshVersion = async () => {
  const candidates = new Set();
  const dshHome = process.env.DSH_HOME || path.join(os.homedir(), ".dsh");
  const profileFlagIndex = process.argv.indexOf("--profile");
  const profileName =
    profileFlagIndex >= 0 && typeof process.argv[profileFlagIndex + 1] === "string"
      ? process.argv[profileFlagIndex + 1]
      : process.argv.find((value) => value.startsWith("--profile="))?.slice(10);

  candidates.add(path.join(dshHome, "profiles", "node_modules", "@deepseek-ai", "dsh", "package.json"));
  if (profileName) {
    candidates.add(
      path.join(
        dshHome,
        "profiles",
        profileName,
        "node_modules",
        "@deepseek-ai",
        "dsh",
        "package.json",
      ),
    );
  }
  candidates.add(path.join(process.cwd(), "node_modules", "@deepseek-ai", "dsh", "package.json"));

  for (const argument of process.argv.slice(1)) {
    if (!path.isAbsolute(argument)) continue;
    let current = path.dirname(argument);
    for (let depth = 0; depth < 10; depth += 1) {
      candidates.add(path.join(current, "package.json"));
      candidates.add(path.join(current, "node_modules", "@deepseek-ai", "dsh", "package.json"));
      const parent = path.dirname(current);
      if (parent === current) break;
      current = parent;
    }
  }

  for (const candidate of candidates) {
    const version = await packageVersion(candidate);
    if (version) return version;
  }
  return "unknown";
};

const dshVersionPromise = detectDshVersion();

const bridgeConfigPath = () =>
  process.env.CODECRAFT_DSH_BRIDGE_CONFIG ||
  (process.platform === "win32"
    ? path.join(process.env.LOCALAPPDATA || os.homedir(), "CodeCraft", "dsh-hook", "bridge.json")
    : path.join(os.homedir(), ".codecraft", "dsh-hook", "bridge.json"));

const truncate = (value, limit = 64 * 1024) => {
  const text = String(value ?? "");
  return text.length > limit ? `${text.slice(0, limit)}...[truncated]` : text;
};

const digest = (value) =>
  createHash("sha256").update(JSON.stringify(value ?? null)).digest("hex");

const sessionIdOf = (value) => {
  const candidates = [
    value?.id,
    value?.sessionId,
    value?.session?.id,
    value?.agent?.session?.id,
    value?.agent?.sessionId,
  ];
  const match = candidates.find((candidate) => typeof candidate === "string" && candidate.length > 0);
  return match || `transient-${pluginInstanceId}`;
};

const workspaceOf = (value) => {
  const candidates = [value?.cwd, value?.session?.header?.cwd, value?.agent?.session?.header?.cwd];
  return candidates.find((candidate) => typeof candidate === "string" && candidate.length > 0) || process.cwd();
};

const textFromContent = (content) => {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .filter((part) => part?.type === "text" && typeof part.text === "string")
    .map((part) => part.text)
    .join("\n");
};

const assistantChunkText = (event) => {
  if (event?.type !== "assistant/chunk") return "";
  const chunk = event?.data?.chunk;
  return chunk?.type === "text-delta" && typeof chunk.text === "string" ? chunk.text : "";
};

const toolSummary = (argumentsValue) => {
  if (typeof argumentsValue === "string") return truncate(argumentsValue, 512);
  try {
    return truncate(JSON.stringify(argumentsValue ?? {}), 512);
  } catch {
    return "[unavailable]";
  }
};

async function readBridgeConfig() {
  try {
    const config = JSON.parse(await readFile(bridgeConfigPath(), "utf8"));
    if (
      config?.protocol !== PROTOCOL ||
      config?.protocolVersion !== PROTOCOL_VERSION ||
      typeof config?.endpoint !== "string" ||
      typeof config?.token !== "string" ||
      typeof config?.bridgeInstanceId !== "string"
    ) return null;
    return config;
  } catch {
    return null;
  }
}

async function exchange(messageType, payload, context = {}, waitForDecision = false, signal) {
  if (signal?.aborted) return { available: true, error: "aborted" };
  const config = await readBridgeConfig();
  if (!config) return { available: false };
  const dshVersion = await dshVersionPromise;
  if (signal?.aborted) return { available: true, error: "aborted" };
  const envelope = {
    protocol: PROTOCOL,
    protocolVersion: PROTOCOL_VERSION,
    messageType,
    messageId: randomUUID(),
    bridgeInstanceId: config.bridgeInstanceId,
    pluginInstanceId,
    pluginVersion: PLUGIN_VERSION,
    dshVersion,
    dshProcessId: process.pid,
    sessionId: context.sessionId || null,
    turnId: context.turnId ?? null,
    stepId: context.stepId ?? null,
    requestId: context.requestId || null,
    capturedAt: Date.now(),
    workspace: context.workspace || process.cwd(),
    capabilities: {
      observation: true,
      toolApproval: true,
      questionAnswer: true,
      planReview: true,
      allowAlways: false,
    },
    token: config.token,
    payload,
  };

  return await new Promise((resolve) => {
    let settled = false;
    let connected = false;
    let response = "";
    let socket;
    try {
      socket = net.createConnection(config.endpoint);
    } catch {
      resolve({ available: false });
      return;
    }
    const finish = (value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      signal?.removeEventListener("abort", onAbort);
      socket.destroy();
      resolve(value);
    };
    const timer = setTimeout(
      () => finish(connected ? { available: true, error: "timeout" } : { available: false }),
      waitForDecision ? DECISION_TIMEOUT_MS : CONNECT_TIMEOUT_MS,
    );
    const onAbort = () => finish({ available: true, error: "aborted" });
    signal?.addEventListener("abort", onAbort, { once: true });
    if (signal?.aborted) {
      onAbort();
      return;
    }
    socket.setEncoding("utf8");
    socket.once("connect", () => {
      connected = true;
      socket.write(`${JSON.stringify(envelope)}\n`);
    });
    socket.on("data", (chunk) => {
      response += chunk;
      if (Buffer.byteLength(response) > MAX_LINE_BYTES) return finish({ available: true, error: "response-too-large" });
      const newline = response.indexOf("\n");
      if (newline < 0) return;
      try {
        finish({ available: true, response: JSON.parse(response.slice(0, newline)) });
      } catch {
        finish({ available: true, error: "invalid-response" });
      }
    });
    socket.once("error", () => finish(connected ? { available: true, error: "disconnected" } : { available: false }));
    socket.once("end", () => {
      if (!settled) finish(connected ? { available: true, error: "disconnected" } : { available: false });
    });
  });
}

export const dshQuestionPayload = (request) => {
  const questions = Array.isArray(request?.questions) ? request.questions.slice(0, 3) : [];
  const planQuestion = questions.find((question) => question?.intent?.kind === "plan-review");
  return {
    requestKind: planQuestion ? "plan" : "question",
    questions: questions.map((question) => ({
      id: String(question?.id || randomUUID()),
      header: typeof question?.header === "string" ? truncate(question.header, 128) : null,
      question: truncate(question?.question || "", 4096),
      detail: typeof question?.detail === "string" ? truncate(question.detail) : null,
      options: Array.isArray(question?.options)
        ? question.options.slice(0, 8).map((option) => ({
            label: truncate(option?.label || "", 256),
            description: typeof option?.description === "string" ? truncate(option.description, 1024) : null,
          }))
        : [],
      multiSelect: question?.multiSelect === true,
      intent: question?.intent?.kind === "plan-review"
        ? { kind: "plan-review", approve: question.intent.approve }
        : null,
    })),
  };
};

export function installUserQuestionBridge(ctx, exchangeFn = exchange) {
  const userQuestions = ctx?.userQuestions;
  const originalProvider = userQuestions?.provider;
  if (!userQuestions || typeof originalProvider?.ask !== "function") return () => {};

  const bridgeProvider = {
    async ask(request) {
      // Agentless calls have no session identity for the CodeCraft store and
      // should retain the provider's existing behavior.
      if (request?.agent === undefined) return originalProvider.ask(request);

      const requestId = randomUUID();
      const outcome = await exchangeFn(
        "request",
        dshQuestionPayload(request),
        {
          requestId,
          sessionId: sessionIdOf(request.agent),
          workspace: workspaceOf(request.agent),
        },
        true,
        request.signal,
      );
      if (!outcome?.available) return originalProvider.ask(request);
      if (outcome.error) {
        if (outcome.error === "aborted") {
          throw new Error("ask_user_question was aborted before the user answered");
        }
        throw new Error(`CodeCraft DSH bridge failed: ${outcome.error}`);
      }
      if (outcome.response?.decision !== "answer" || !Array.isArray(outcome.response?.answers)) {
        throw new Error(outcome.response?.reason || "CodeCraft did not return a DSH answer");
      }
      return { answers: outcome.response.answers };
    },
  };

  // UserQuestionService intentionally exposes only one registration slot. The
  // Web provider is already registered by dsh-host-apiproxy, so replace the
  // runtime slot for this plugin's lifetime and restore it on disposal.
  userQuestions.provider = bridgeProvider;
  return () => {
    if (userQuestions.provider === bridgeProvider) userQuestions.provider = originalProvider;
  };
}

const eventContext = (session, event) => ({
  sessionId: sessionIdOf(session),
  workspace: workspaceOf(session),
  turnId: event?.data?.turn,
  stepId: event?.data?.step,
});

export const name = "codecraft-dsh-plugin";

export function apply(ctx) {
  const heartbeat = () => void exchange("heartbeat", {
    nodeVersion: process.version,
    platform: process.platform,
  });
  heartbeat();
  const heartbeatTimer = setInterval(heartbeat, HEARTBEAT_INTERVAL_MS);
  heartbeatTimer.unref?.();
  ctx.effect?.(() => () => {
    clearInterval(heartbeatTimer);
    void exchange("shutdown", { reason: "plugin-disposed" });
  });

  ctx.on("session/event", (session, event) => {
    const context = eventContext(session, event);
    if (event?.type === "tool/call") {
      const call = {
        sessionId: context.sessionId,
        workspace: context.workspace,
        name: event?.data?.name || "tool",
        arguments: event?.data?.arguments || "{}",
      };
      toolCalls.set(String(event?.data?.callId || ""), call);
    }
    if (event?.type === "tool/result") {
      const callId = event?.data?.message?.toolCallId || event?.data?.message?.callId;
      if (callId) toolCalls.delete(String(callId));
    }
    const displayText = assistantChunkText(event);
    const userText = event?.type === "user/message" ? textFromContent(event?.data?.content) : "";
    void exchange("event", {
      event,
      displayText: displayText ? truncate(displayText) : null,
      userText: userText ? truncate(userText, 4096) : null,
    }, context);
  });

  ctx.on("tools/pre-execute", async (exec, next) => {
    void exchange("event", {
      event: {
        type: "codecraft/tool-pre-execute",
        data: {
          callId: exec?.callId,
          rootCallId: exec?.rootCallId,
          name: exec?.name,
          argumentsDigest: digest(exec?.arguments),
          argumentsSummary: toolSummary(exec?.arguments),
        },
      },
    }, {
      sessionId: sessionIdOf(exec?.agent),
      workspace: workspaceOf(exec?.agent),
    });
    return next();
  });

  ctx.on("tools/result", (exec, result) => {
    void exchange("event", {
      event: {
        type: "codecraft/tool-result",
        data: {
          callId: exec?.callId,
          name: exec?.name,
          isError: result?.isError === true,
        },
      },
    }, {
      sessionId: sessionIdOf(exec?.agent),
      workspace: workspaceOf(exec?.agent),
    });
  });

  ctx.on("approval/request", async (request, next) => {
    const requestId = randomUUID();
    const call = toolCalls.get(String(request?.callId || ""));
    const outcome = await exchange("request", {
      requestKind: "approval",
      toolName: request?.toolName || call?.name || "tool",
      callId: request?.callId || null,
      reason: request?.reason || null,
      inputDigest: digest(call?.arguments),
      inputSummary: toolSummary(call?.arguments),
    }, {
      requestId,
      sessionId: sessionIdOf(request?.agent),
      workspace: call?.workspace || workspaceOf(request?.agent),
    }, true, request?.signal);
    if (!outcome.available) return next();
    if (outcome.response?.decision === "allowOnce") return "allowed-once";
    if (outcome.response?.decision === "deny") return "rejected";
    return "rejected";
  });

  // apiProxy owns the default Web provider and is composed after userQuestions.
  // Wait for both services so this plugin never snapshots an empty provider
  // during the composition window.
  ctx.inject?.(["apiProxy", "userQuestions"], (questionCtx) => {
    const disposeQuestionBridge = installUserQuestionBridge(questionCtx);
    questionCtx.effect?.(
      () => () => disposeQuestionBridge(),
      "codecraft-dsh-plugin: user-questions provider",
    );
  });
}
