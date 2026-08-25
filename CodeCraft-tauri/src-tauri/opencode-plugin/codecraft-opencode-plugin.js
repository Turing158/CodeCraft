import { createHash, randomUUID } from "node:crypto";
import {
  mkdir,
  readFile,
  readdir,
  rename,
  stat,
  unlink,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";

const PROTOCOL_VERSION = "0.4";
const PLUGIN_VERSION = "0.4.8";
const HEARTBEAT_INTERVAL_MS = 5000;
const APP_HEARTBEAT_STALE_MS = 20000;
const DECISION_POLL_MS = 200;
const GATE_TIMEOUT_MS = 300000;
const SESSION_ACTION_RETRY_DELAYS_MS = [100, 250, 500, 1000, 1500];
const MAX_TEXT = 64 * 1024;
const MAX_ENVELOPE_BYTES = 256 * 1024;
const CONTROL_TOOLS = new Set(["question"]);

const instanceID = randomUUID();
const sessionRules = new Map();
const gateWaiters = new Map();
const permissionRequests = new Map();
const gateDecisionByCall = new Map();
const nativeDecisionByCall = new Map();
const policyDecisionByCall = new Map();
const sessionAgents = new Map();
const assistantMessageAgents = new Map();
const controlTools = new Map();
let writeSequence = 0;
let context;


function baseDataDir() {
  if (process.env.CODECRAFT_OPENCODE_HOOK_DIR) {
    return path.resolve(process.env.CODECRAFT_OPENCODE_HOOK_DIR);
  }
  if (process.env.LOCALAPPDATA)
    return path.join(process.env.LOCALAPPDATA, "CodeCraft", "opencode-hook");
  return path.join(
    process.env.USERPROFILE ?? process.env.HOME ?? os.homedir(),
    ".codecraft",
    "opencode-hook",
  );
}

const root = baseDataDir();
const controlDir = path.join(root, "control");
const instanceDir = path.join(root, "instances");
const inboxDir = path.join(root, "inbox");
const outboxDir = path.join(root, "outbox");
const processingDir = path.join(root, "processing");
const appHeartbeatPath = path.join(path.dirname(root), "app-running");

function clip(value, limit = MAX_TEXT) {
  if (typeof value !== "string") return value;
  return value.length <= limit
    ? value
    : `${value.slice(0, limit)}...[truncated]`;
}

function permissionWithSafetyGate(permission) {
  if (permission === "deny") return "deny";
  const current =
    permission && typeof permission === "object" && !Array.isArray(permission)
      ? { ...permission }
      : {};
  for (const key of ["edit", "write", "bash"]) {
    if (current[key] === "deny") continue;
    if (
      current[key] &&
      typeof current[key] === "object" &&
      !Array.isArray(current[key])
    ) {
      current[key] = { "*": "ask", ...current[key] };
      continue;
    }
    current[key] = "ask";
  }
  return current;
}

function configureSafetyPermissions(config) {
  config.permission = permissionWithSafetyGate(config.permission);
  config.agent ??= {};
  config.agent.build ??= {};
  config.agent.build.permission = permissionWithSafetyGate(
    config.agent.build.permission,
  );
}

function safeObject(value, depth = 0, maxDepth = 3) {
  if (value === undefined) return undefined;
  if (value === null || typeof value === "boolean" || typeof value === "number")
    return value;
  if (typeof value === "string") return clip(value);
  if (depth >= maxDepth)
    return Array.isArray(value) ? `[array:${value.length}]` : "[object]";
  if (Array.isArray(value)) {
    return value
      .slice(0, 40)
      .map((item) => safeObject(item, depth + 1, maxDepth));
  }
  if (!value || typeof value !== "object") return String(value);
  return Object.fromEntries(
    Object.entries(value)
      .slice(0, 64)
      .filter(
        ([key]) =>
          !/(authorization|cookie|secret|token|password|api[-_]?key|credential|environment|headers?)/i.test(
            key,
          ),
      )
      .map(([key, item]) => [key, safeObject(item, depth + 1, maxDepth)]),
  );
}

function stableValue(value) {
  if (Array.isArray(value)) return value.map(stableValue);
  if (!value || typeof value !== "object") return value;
  return Object.fromEntries(
    Object.keys(value)
      .sort()
      .map((key) => [key, stableValue(value[key])]),
  );
}

function gateRuleKey(tool, args) {
  return createHash("sha256")
    .update(JSON.stringify([tool, stableValue(args)]))
    .digest("hex");
}

function commandText(input) {
  if (typeof input === "string") return input;
  return typeof input?.command === "string" ? input.command : undefined;
}

function commandRisk(command) {
  const normalized = String(command ?? "")
    .toLowerCase()
    .trim()
    .split(/\s+/)
    .join(" ");
  if (!normalized) return "elevated";

  const padded = ` ${normalized} `;
  const highRiskMarkers = [
    "rm -rf",
    "remove-item",
    " del ",
    " rmdir ",
    "rd /s",
    "format ",
    "diskpart",
    "git reset --hard",
    "git clean -f",
    "git checkout --",
    "git restore ",
    "git push --force",
    "git push -f",
    "git branch -d",
    "drop table",
    "truncate table",
    "delete from",
    "shutdown",
    "reboot",
    "stop-process",
    "taskkill",
    "kill -9",
  ];
  if (highRiskMarkers.some((marker) => padded.includes(marker))) return "high";

  const elevatedMarkers = [
    ">",
    "|",
    "&&",
    ";",
    "sudo ",
    "runas ",
    "set-content",
    "add-content",
    "new-item",
    "move-item",
    "copy-item",
    "git add",
    "git commit",
    "git push",
    "git merge",
    "git rebase",
    "npm install",
    "cargo install",
  ];
  if (elevatedMarkers.some((marker) => normalized.includes(marker))) {
    return "elevated";
  }

  const lowRiskPrefixes = [
    "pwd",
    "cd ",
    "ls",
    "dir",
    "get-childitem",
    "get-location",
    "get-content",
    "select-string",
    "findstr",
    "rg",
    "grep",
    "where",
    "where.exe",
    "which",
    "type ",
    "cat ",
    "head ",
    "tail ",
    "git status",
    "git diff",
    "git log",
    "git show",
    "git branch",
    "git remote",
    "git rev-parse",
    "git ls-files",
    "cargo metadata",
    "cargo tree",
    "npm view",
    "npm list",
    "node --version",
    "python --version",
  ];
  const isLowRiskSegment = (segment) =>
    lowRiskPrefixes.some(
      (prefix) =>
        segment === prefix ||
        (prefix.endsWith(" ") && segment.startsWith(prefix)) ||
        segment.startsWith(`${prefix} `),
    );
  const segments = normalized
    .split(/[,\n]/)
    .map((segment) => segment.trim())
    .filter(Boolean);
  if (segments.length > 1 && segments.every(isLowRiskSegment)) return "low";
  return isLowRiskSegment(normalized) ? "low" : "elevated";
}

function toolRisk(tool, input) {
  switch (String(tool ?? "").toLowerCase()) {
    case "read":
    case "glob":
    case "grep":
    case "websearch":
    case "brainstorm":
      return "low";
    case "bash":
    case "shell":
    case "exec":
    case "command": {
      const command = commandText(input);
      return command === undefined ? "elevated" : commandRisk(command);
    }
    default:
      return "elevated";
  }
}

function requiresUserDecision(tool) {
  const normalized = String(tool ?? "")
    .toLowerCase()
    .replace(/[^a-z0-9]/g, "");
  return new Set([
    "askuserquestion",
    "requestuserinput",
    "question",
    "plan",
    "planexit",
    "exitplanmode",
    "updateplan",
  ]).has(normalized);
}

function shouldAutoApprove(approvalMode, tool, input) {
  if (requiresUserDecision(tool)) return false;
  if (approvalMode === "automatic") return true;
  return approvalMode === "risk" && toolRisk(tool, input) === "low";
}

async function ensureDirectories() {
  await Promise.all(
    [controlDir, instanceDir, inboxDir, outboxDir, processingDir].map(
      (directory) => mkdir(directory, { recursive: true }),
    ),
  );
}

async function atomicJson(directory, name, value) {
  await mkdir(directory, { recursive: true });
  const serialized = JSON.stringify(value);
  if (Buffer.byteLength(serialized, "utf8") > MAX_ENVELOPE_BYTES) {
    throw new Error("CodeCraft OpenCode IPC envelope exceeded its size limit");
  }
  writeSequence += 1;
  const temporary = path.join(
    directory,
    `.${name}.${process.pid}.${writeSequence}.tmp`,
  );
  const target = path.join(directory, name);
  await writeFile(temporary, serialized, { encoding: "utf8", flag: "wx" });
  try {
    await rename(temporary, target);
  } catch (error) {
    await unlink(temporary).catch(() => {});
    throw error;
  }
}

async function appendInbox(kind, payload, maxDepth = 3) {
  const now = Date.now();
  const name = `${String(now).padStart(20, "0")}-${instanceID}-${randomUUID()}.json`;
  await atomicJson(inboxDir, name, {
    protocolVersion: PROTOCOL_VERSION,
    pluginVersion: PLUGIN_VERSION,
    pluginInstanceID: instanceID,
    capturedAt: now,
    kind,
    payload: safeObject(payload, 0, maxDepth),
  });
}

async function readJson(file) {
  return JSON.parse(await readFile(file, "utf8"));
}

async function controlState() {
  try {
    const value = await readJson(path.join(controlDir, "enabled.json"));
    if (value?.protocolVersion !== PROTOCOL_VERSION)
      throw new Error("CodeCraft protocol mismatch");
    return value;
  } catch (error) {
    if (error?.code === "ENOENT")
      return { enabled: false, approvalMode: "manual" };
    throw error;
  }
}

async function appIsRunning() {
  try {
    const info = await stat(appHeartbeatPath);
    return Date.now() - info.mtimeMs <= APP_HEARTBEAT_STALE_MS;
  } catch {
    return false;
  }
}

function authorizationHeader() {
  const password = process.env.OPENCODE_SERVER_PASSWORD;
  if (!password) return undefined;
  const username = process.env.OPENCODE_SERVER_USERNAME || "opencode";
  return `Basic ${Buffer.from(`${username}:${password}`, "utf8").toString("base64")}`;
}

function expandRoute(route, params) {
  return route.replace(/{([^{}]+)}/g, (match, key) => {
    const value = params?.[key];
    return value === undefined || value === null
      ? match
      : encodeURIComponent(String(value));
  });
}

function requestError(route, status, payload) {
  const detail =
    (typeof payload === "string" && payload) ||
    (typeof payload?.data?.message === "string" && payload.data.message) ||
    (typeof payload?.message === "string" && payload.message) ||
    (typeof payload?._tag === "string" && payload._tag) ||
    (payload === undefined ? "" : JSON.stringify(payload));
  const error = new Error(
    `OpenCode ${route} failed${status ? ` (HTTP ${status})` : ""}${detail ? `: ${clip(detail, 1000)}` : ""}`,
  );
  error.status = status;
  return error;
}

function delay(durationMs) {
  return new Promise((resolve) => setTimeout(resolve, durationMs));
}

function retryableSessionActionError(error) {
  return error?.status === 409;
}

async function retrySessionAction(action) {
  let lastError;
  for (let attempt = 0; attempt <= SESSION_ACTION_RETRY_DELAYS_MS.length; attempt += 1) {
    try {
      return await action();
    } catch (error) {
      lastError = error;
      if (
        !retryableSessionActionError(error) ||
        attempt === SESSION_ACTION_RETRY_DELAYS_MS.length
      ) {
        throw error;
      }
      await delay(SESSION_ACTION_RETRY_DELAYS_MS[attempt]);
    }
  }
  throw lastError;
}

// The injected SDK client talks to the current OpenCode instance even when it
// runs without a listening HTTP server, so it is the primary transport. Direct
// localhost HTTP stays as the fallback for hosts that do not inject a client.
async function instanceRequest(method, route, params, body) {
  const client = context?.client?._client;
  const verb = method.toLowerCase();
  if (client && typeof client[verb] === "function") {
    const result = await client[verb]({
      url: route,
      path: params,
      body,
      headers:
        body === undefined ? undefined : { "Content-Type": "application/json" },
    });
    const status = result?.response?.status;
    if (
      result?.error !== undefined ||
      (typeof status === "number" && status >= 400)
    ) {
      throw requestError(expandRoute(route, params), status, result?.error);
    }
    return result?.data;
  }
  return httpRequest(method, expandRoute(route, params), body);
}

async function switchSessionAgent(sessionID, agent) {
  await retrySessionAction(() =>
    instanceRequest(
      "POST",
      "/api/session/{sessionID}/agent",
      { sessionID },
      { agent },
    ),
  );
}

async function sendSessionMessage(sessionID, text) {
  await retrySessionAction(() =>
    instanceRequest(
      "POST",
      "/api/session/{sessionID}/prompt",
      { sessionID },
      {
        prompt: { text },
        delivery: "queue",
        resume: true,
      },
    ),
  );
}

async function httpRequest(method, route, body) {
  const url = new URL(route, context.serverUrl);
  const headers = new Headers({
    "x-opencode-directory": encodeURIComponent(context.directory),
  });
  const authorization = authorizationHeader();
  if (authorization) headers.set("authorization", authorization);
  if (body !== undefined) headers.set("content-type", "application/json");
  let response;
  try {
    response = await fetch(url, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch (error) {
    throw new Error(
      `OpenCode ${route} is unreachable at ${url.origin}: ${clip(String(error?.message ?? error), 500)}`,
    );
  }
  if (!response.ok) {
    const detail = clip(await response.text().catch(() => ""), 1000);
    throw requestError(route, response.status, detail);
  }
  if (response.status === 204) return undefined;
  return response.json().catch(() => undefined);
}

async function writeInstanceHeartbeat() {
  const enabled = await controlState().catch(() => ({ enabled: false }));
  await atomicJson(instanceDir, `${instanceID}.json`, {
    protocolVersion: PROTOCOL_VERSION,
    pluginVersion: PLUGIN_VERSION,
    pluginInstanceID: instanceID,
    integrationChannel: "localhostHttpBridge",
    decisionTransport: context?.client?._client ? "sdkClient" : "localhostHttp",
    processID: process.pid,
    startedAt: startedAt,
    heartbeat: Date.now(),
    directory: clip(context.directory),
    worktree: clip(context.worktree),
    serverProtocol: new URL(context.serverUrl).protocol,
    enabledGeneration: enabled.generation ?? null,
    capabilities: {
      question: { list: true, reply: true, reject: true },
      permission: { list: true, once: true, always: true, reject: true },
      session: { switchAgent: true, prompt: true },
      toolHooks: {
        before: true,
        after: true,
        asyncBefore: true,
        rejectError: true,
      },
      credentialsWrittenToIpc: false,
    },
  });
}

async function writeReceipt(decision, result, error) {
  await appendInbox("receipt", {
    decisionID: decision.decisionID,
    reviewID: decision.reviewID,
    sessionID: decision.sessionID,
    requestID: decision.requestID,
    decisionType: decision.type,
    result,
    error: error ? clip(String(error), 2000) : undefined,
  });
}

async function applyDecision(decision) {
  if (
    decision?.protocolVersion !== PROTOCOL_VERSION ||
    decision?.pluginInstanceID !== instanceID ||
    typeof decision?.decisionID !== "string"
  ) {
    throw new Error("Invalid CodeCraft OpenCode decision envelope");
  }

  if (decision.type === "question") {
    if (decision.action === "reject") {
      await instanceRequest("POST", "/question/{requestID}/reject", {
        requestID: decision.requestID,
      });
    } else {
      await instanceRequest(
        "POST",
        "/question/{requestID}/reply",
        { requestID: decision.requestID },
        { answers: decision.answers },
      );
    }
    return;
  }
  if (decision.type === "permission") {
    const pending = permissionRequests.get(decision.requestID);
    await instanceRequest(
      "POST",
      "/permission/{requestID}/reply",
      { requestID: decision.requestID },
      {
        reply: decision.action,
        ...(decision.message ? { message: clip(decision.message, 2000) } : {}),
      },
    );
    permissionRequests.delete(decision.requestID);
    if (pending?.callID) {
      nativeDecisionByCall.set(pending.callID, {
        action: decision.action,
        sessionID: pending.sessionID,
      });
    }
    return;
  }
  if (decision.type === "strictToolGate") {
    const waiter = gateWaiters.get(decision.reviewID);
    if (!waiter || waiter.sessionID !== decision.sessionID)
      throw new Error("Tool gate is no longer pending");
    gateWaiters.delete(decision.reviewID);
    if (waiter.callID) {
      gateDecisionByCall.set(waiter.callID, {
        action: decision.action,
        sessionID: decision.sessionID,
      });
    }
    waiter.resolve(decision.action);
    return;
  }
  if (decision.type === "switchAgent") {
    await switchSessionAgent(decision.sessionID, decision.agent);
    sessionAgents.set(decision.sessionID, decision.agent);
    return;
  }
  if (decision.type === "sessionMessage") {
    await sendSessionMessage(decision.sessionID, clip(decision.text, MAX_TEXT));
    return;
  }
  throw new Error("Unsupported CodeCraft OpenCode decision type");
}

async function consumeDecisions() {
  const files = await readdir(outboxDir).catch(() => []);
  for (const name of files
    .filter((item) => item.endsWith(".json"))
    .slice(0, 100)) {
    const source = path.join(outboxDir, name);
    let decision;
    try {
      decision = await readJson(source);
    } catch {
      continue;
    }
    if (decision?.pluginInstanceID !== instanceID) continue;
    const claimed = path.join(processingDir, `${instanceID}-${name}`);
    try {
      await rename(source, claimed);
    } catch {
      continue;
    }
    try {
      await applyDecision(decision);
      await writeReceipt(decision, "applied");
    } catch (error) {
      await writeReceipt(decision, "error", error);
      const waiter = gateWaiters.get(decision.reviewID);
      if (waiter) {
        gateWaiters.delete(decision.reviewID);
        waiter.reject(error);
      }
    } finally {
      await unlink(claimed).catch(() => {});
    }
  }
}

async function rejectUnavailableWaiters() {
  const enabled = await controlState()
    .then((value) => value.enabled === true)
    .catch(() => true);
  const running = await appIsRunning();
  if (enabled && running) return;
  for (const [reviewID, waiter] of gateWaiters) {
    gateWaiters.delete(reviewID);
    waiter.reject(
      new Error(
        enabled
          ? "CodeCraft is offline"
          : "CodeCraft OpenCode integration was disabled",
      ),
    );
  }
}

async function waitForGate(reviewID, sessionID, callID) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      gateWaiters.delete(reviewID);
      reject(new Error("CodeCraft tool approval timed out"));
    }, GATE_TIMEOUT_MS);
    gateWaiters.set(reviewID, {
      sessionID,
      callID,
      resolve(value) {
        clearTimeout(timer);
        resolve(value);
      },
      reject(error) {
        clearTimeout(timer);
        reject(error);
      },
    });
  });
}

function eventPayload(event) {
  const properties = event?.properties ?? {};
  if (event?.type === "question.asked") {
    return {
      eventType: event.type,
      requestID: properties.id,
      sessionID: properties.sessionID,
      questions: properties.questions,
      tool: properties.tool,
    };
  }
  if (event?.type === "permission.asked") {
    return {
      eventType: event.type,
      requestID: properties.id,
      sessionID: properties.sessionID,
      permission: properties.permission,
      patterns: properties.patterns,
      always: properties.always,
      metadata: properties.metadata,
      tool: properties.tool,
    };
  }
  if (
    event?.type === "question.replied" ||
    event?.type === "question.rejected" ||
    event?.type === "permission.replied"
  ) {
    return {
      eventType: event.type,
      properties: {
        requestID: properties.requestID ?? properties.id,
        sessionID: properties.sessionID,
      },
    };
  }
  if (event?.type === "session.status") {
    return {
      eventType: event.type,
      properties: {
        sessionID: properties.sessionID,
        status: { type: properties.status?.type },
      },
    };
  }
  if (event?.type === "session.idle") {
    return {
      eventType: event.type,
      properties: { sessionID: properties.sessionID },
    };
  }
  return { eventType: event?.type };
}

function controlToolKey(sessionID, callID) {
  return `${sessionID ?? ""}:${callID ?? ""}`;
}

function firstQuestion(request) {
  return Array.isArray(request?.questions) ? request.questions[0] : undefined;
}

function optionLabels(question) {
  if (!Array.isArray(question?.options)) return [];
  return question.options.map((option) =>
    typeof option === "string" ? option : option?.label,
  );
}

function strictPlanQuestion(request, agent) {
  const question = firstQuestion(request);
  const labels = optionLabels(question);
  return (
    agent === "plan" &&
    question?.header === "Build Agent" &&
    labels.length === 2 &&
    labels[0] === "Yes" &&
    labels[1] === "No"
  );
}

const startedAt = Date.now();

async function codecraftOpenCodePlugin(input) {
  context = input;
  await ensureDirectories();
  await writeInstanceHeartbeat();

  const decisionTimer = setInterval(() => {
    void Promise.allSettled([consumeDecisions(), rejectUnavailableWaiters()]);
  }, DECISION_POLL_MS);
  decisionTimer.unref?.();
  const heartbeatTimer = setInterval(() => {
    void Promise.allSettled([writeInstanceHeartbeat()]);
  }, HEARTBEAT_INTERVAL_MS);
  heartbeatTimer.unref?.();
  let lastHeartbeat = Date.now();

  return {
    async config(config) {
      configureSafetyPermissions(config);
    },
    async "chat.message"(inputValue) {
      if (
        typeof inputValue?.sessionID === "string" &&
        typeof inputValue?.agent === "string"
      ) {
        sessionAgents.set(inputValue.sessionID, inputValue.agent);
      }
      if (typeof inputValue?.sessionID === "string") {
        await appendInbox("sessionInput", {
          sessionID: inputValue.sessionID,
          messageID: inputValue.messageID,
          agent: inputValue.agent,
        });
      }
    },
    async "experimental.text.complete"(inputValue, output) {
      const agent =
        assistantMessageAgents.get(inputValue.messageID) ??
        inputValue.agent ??
        sessionAgents.get(inputValue.sessionID);
      await appendInbox("assistantMessage", {
        sessionID: inputValue.sessionID,
        messageID: inputValue.messageID,
        partID: inputValue.partID,
        agent,
        text: output.text,
      });
      assistantMessageAgents.delete(inputValue.messageID);
    },
    async event({ event }) {
      if (Date.now() - lastHeartbeat >= HEARTBEAT_INTERVAL_MS) {
        lastHeartbeat = Date.now();
        await writeInstanceHeartbeat();
      }
      if (event?.type === "permission.asked") {
        const properties = event.properties ?? {};
        const callID = properties.tool?.callID;
        permissionRequests.set(properties.id, {
          callID,
          sessionID: properties.sessionID,
          always: Array.isArray(properties.always) ? properties.always : [],
        });
        const control = await controlState();
        const policyDecision = callID
          ? policyDecisionByCall.get(callID)
          : undefined;
        const permissionTool = properties.tool?.name ?? properties.permission;
        const permissionInput =
          properties.tool?.args ??
          properties.metadata?.args ??
          (properties.permission === "bash" && Array.isArray(properties.patterns)
            ? { command: properties.patterns.join(", ") }
            : properties.metadata);
        const policyApplies =
          policyDecision?.sessionID === properties.sessionID ||
          (control.enabled === true &&
            shouldAutoApprove(control.approvalMode, permissionTool, permissionInput));
        if (policyApplies) {
          await instanceRequest(
            "POST",
            "/permission/{requestID}/reply",
            { requestID: properties.id },
            { reply: "once" },
          );
          permissionRequests.delete(properties.id);
          if (callID) policyDecisionByCall.delete(callID);
          await appendInbox("nativePermissionAutoResolved", {
            sessionID: properties.sessionID,
            requestID: properties.id,
            callID,
            source: `approvalPolicy:${policyDecision?.approvalMode ?? control.approvalMode}`,
          });
          return;
        }
        const gateDecision = callID
          ? gateDecisionByCall.get(callID)
          : undefined;
        if (gateDecision && gateDecision.sessionID === properties.sessionID) {
          const canAlways =
            Array.isArray(properties.always) && properties.always.length > 0;
          await instanceRequest(
            "POST",
            "/permission/{requestID}/reply",
            { requestID: properties.id },
            {
              reply:
                gateDecision.action === "reject"
                  ? "reject"
                  : gateDecision.action === "allowSession" && canAlways
                    ? "always"
                    : "once",
            },
          );
          await appendInbox("nativePermissionAutoResolved", {
            sessionID: properties.sessionID,
            requestID: properties.id,
            callID,
            source: "strictToolGate",
          });
          return;
        }
      }
      if (event?.type === "permission.replied") {
        permissionRequests.delete(event.properties?.requestID);
      }
      if (event?.type === "message.updated") {
        const info = event.properties?.info;
        if (info?.role === "assistant" && typeof info.id === "string") {
          assistantMessageAgents.set(info.id, info.agent);
          if (
            typeof info.sessionID === "string" &&
            typeof info.agent === "string"
          ) {
            sessionAgents.set(info.sessionID, info.agent);
          }
        }
      }
      if (event?.type === "question.asked") {
        const properties = event.properties ?? {};
        const agent = sessionAgents.get(properties.sessionID);
        if (strictPlanQuestion(properties, agent)) return;
        await appendInbox("event", eventPayload(event), 5);
        return;
      }
      await appendInbox("event", eventPayload(event), 3);
    },
    "tool.execute.before": async (toolInput, output) => {
      if (toolInput.tool === "plan_exit") return;
      if (CONTROL_TOOLS.has(toolInput.tool)) {
        controlTools.set(controlToolKey(toolInput.sessionID, toolInput.callID), {
          tool: toolInput.tool,
          args: output?.args ?? {},
          messageID: toolInput.messageID,
        });
        await appendInbox("controlTool", {
          sessionID: toolInput.sessionID,
          callID: toolInput.callID,
          tool: toolInput.tool,
          args: output?.args ?? {},
          messageID: toolInput.messageID,
        });
        return;
      }
      const control = await controlState();
      if (control.enabled !== true) return;
      if (!(await appIsRunning()))
        throw new Error("CodeCraft is offline; tool execution was denied");

      if (shouldAutoApprove(control.approvalMode, toolInput.tool, output?.args)) {
        policyDecisionByCall.set(toolInput.callID, {
          approvalMode: control.approvalMode,
          sessionID: toolInput.sessionID,
        });
        return;
      }

      const rule = gateRuleKey(toolInput.tool, output?.args ?? {});
      const gateDecision = gateDecisionByCall.get(toolInput.callID);
      if (gateDecision && gateDecision.sessionID === toolInput.sessionID) {
        if (gateDecision.action === "reject") {
          throw new Error("CodeCraft denied this tool call");
        }
        if (gateDecision.action === "allowSession") {
          const rules = sessionRules.get(toolInput.sessionID) ?? new Set();
          rules.add(rule);
          sessionRules.set(toolInput.sessionID, rules);
        }
        return;
      }
      const nativeDecision = nativeDecisionByCall.get(toolInput.callID);
      if (nativeDecision && nativeDecision.sessionID === toolInput.sessionID) {
        if (nativeDecision.action === "reject")
          throw new Error("OpenCode permission denied this tool call");
        if (nativeDecision.action === "always") {
          const rules = sessionRules.get(toolInput.sessionID) ?? new Set();
          rules.add(rule);
          sessionRules.set(toolInput.sessionID, rules);
        }
        return;
      }
      const allowed = sessionRules.get(toolInput.sessionID);
      if (allowed?.has(rule)) return;

      const reviewID = randomUUID();
      await appendInbox("pending", {
        reviewType: "strictToolGate",
        reviewID,
        sessionID: toolInput.sessionID,
        callID: toolInput.callID,
        tool: toolInput.tool,
        args: output?.args ?? {},
        ruleKey: rule,
      });
      const action = await waitForGate(
        reviewID,
        toolInput.sessionID,
        toolInput.callID,
      );
      if (action === "allowSession") {
        const rules = sessionRules.get(toolInput.sessionID) ?? new Set();
        rules.add(rule);
        sessionRules.set(toolInput.sessionID, rules);
        return;
      }
      if (action === "allowOnce") return;
      throw new Error("CodeCraft denied this tool call");
    },
    "tool.execute.after": async (toolInput, output) => {
      gateDecisionByCall.delete(toolInput.callID);
      nativeDecisionByCall.delete(toolInput.callID);
      policyDecisionByCall.delete(toolInput.callID);
      await appendInbox("toolAfter", {
        sessionID: toolInput.sessionID,
        callID: toolInput.callID,
        tool: toolInput.tool,
        title: output?.title,
        outputLength:
          typeof output?.output === "string" ? output.output.length : undefined,
      });
    },
  };
}

// OpenCode loads every named module export as a plugin hook. Keep the runtime
// entrypoint as the only export; attach test helpers to the function instead
// so the loader cannot try to invoke them as hooks.
export default Object.assign(codecraftOpenCodePlugin, {
  commandRisk,
  retryableSessionActionError,
  retrySessionAction,
  requiresUserDecision,
  shouldAutoApprove,
  toolRisk,
});
