// Runs the installed CLI against a local deterministic model and isolated home.
import { createServer } from "node:http";
import { spawn, spawnSync } from "node:child_process";
import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
import { randomUUID } from "node:crypto";
import { createInterface } from "node:readline";

async function runAcpPlan(child, cwd, prompt) {
  let id = 0;
  const pending = new Map();
  const lines = createInterface({ input: child.stdout });
  const send = (message) => child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", ...message })}\n`);
  const request = (method, params) => new Promise((resolve, reject) => {
    const requestId = ++id;
    pending.set(requestId, { resolve, reject });
    send({ id: requestId, method, params });
  });
  lines.on("line", (line) => {
    let message;
    try { message = JSON.parse(line); } catch { return; }
    if (message.method && message.id !== undefined) {
      const option = message.params?.options?.find((option) => option.kind === "allow_once");
      if (message.method === "session/request_permission" && option) {
        send({ id: message.id, result: { outcome: { outcome: "selected", optionId: option.optionId } } });
      } else send({ id: message.id, error: { code: -32601, message: "Unsupported fixture client method" } });
    } else if (pending.has(message.id)) {
      const handler = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) handler.reject(new Error(JSON.stringify(message.error)));
      else handler.resolve(message.result);
    }
  });
  child.on("close", () => {
    for (const handler of pending.values()) handler.reject(new Error("Kimi ACP exited before replying"));
    pending.clear();
    lines.close();
  });
  await request("initialize", { protocolVersion: 1, clientCapabilities: {}, clientInfo: { name: "codecraft-hook-probe", version: "1" } });
  const session = await request("session/new", { cwd, mcpServers: [] });
  await request("session/prompt", { sessionId: session.sessionId, prompt: [{ type: "text", text: prompt }] });
  child.stdin.end();
}

const script = fileURLToPath(import.meta.url);
if (process.argv[2] === "--capture") {
  const chunks = [];
  for await (const chunk of process.stdin) chunks.push(chunk);
  const payload = JSON.parse(Buffer.concat(chunks).toString("utf8"));
  await writeFile(join(process.argv[3], `${Date.now()}-${randomUUID()}.json`), JSON.stringify(payload, null, 2));
} else {
  const kimi = process.argv[2];
  const adapter = process.argv[3];
  const scenario = process.argv[4] ?? "read";
  if (!kimi) throw new Error("Pass the absolute path to kimi.exe");
  if (!["read", "question", "plan"].includes(scenario)) throw new Error("Scenario must be read, question, or plan");
  const root = resolve(dirname(script), "../src-tauri/target/kimi-probe", randomUUID());
  const inbox = adapter ? join(root, "CodeCraft", "kimi-hook", "inbox") : join(root, "events");
  await mkdir(inbox, { recursive: true });
  await writeFile(join(root, "fixture.txt"), "codecraft-kimi-fixture\n");
  const events = ["SessionStart", "UserPromptSubmit", "TurnStarted", "PreToolUse", "PostToolUse", "PostToolUseFailure", "PermissionRequest", "PermissionResult", "Stop", "StopFailure", "SessionEnd"];
  const questions = [{ header: "Format", question: "Which output format?", multi_select: false,
    options: [{ label: "JSON", description: "Structured data" }, { label: "Text", description: "Plain text" }] }];
  const plan = "# Fixture plan\n\n1. Read fixture.txt.\n2. Report the result.\n";
  const prompt = scenario === "question" ? "Ask which output format to use with AskUserQuestion, then finish."
    : scenario === "plan" ? "Enter plan mode, write a plan to its plan file, then call ExitPlanMode."
      : "Read fixture.txt once, then finish.";
  let requests = 0;
  let modelError;
  const server = createServer(async (req, res) => {
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    const input = JSON.parse(Buffer.concat(chunks).toString("utf8"));
    const results = input.messages?.filter((message) => message.role === "tool") ?? [];
    let call;
    if (scenario === "plan") {
      if (results.length === 0) call = { name: "EnterPlanMode", arguments: {} };
      else if (results.length === 1) {
        const path = /Plan file: ([^\r\n]+)/.exec(results[0].content)?.[1]?.trim();
        const planRelative = path ? relative(root, resolve(path)) : "..";
        if (path && !isAbsolute(planRelative) && planRelative !== ".." && !planRelative.startsWith(`..\\`) && !planRelative.startsWith("../")) call = { name: "Write", arguments: { path, content: plan } };
        else modelError = "Kimi did not supply an isolated plan file";
      } else if (results.length === 2) call = { name: "ExitPlanMode", arguments: {} };
    } else if (results.length === 0) {
      call = scenario === "question" ? { name: "AskUserQuestion", arguments: { questions } }
        : { name: "Read", arguments: { path: join(root, "fixture.txt") } };
    }
    const done = !call;
    requests++;
    const delta = done ? { role: "assistant", content: "Fixture complete." } : {
      role: "assistant", content: null, tool_calls: [{ index: 0, id: `fixture-${scenario}-${results.length + 1}`, type: "function", function: { name: call.name, arguments: JSON.stringify(call.arguments) } }],
    };
    if (input.stream) {
      res.writeHead(200, { "Content-Type": "text/event-stream" });
      res.write(`data: ${JSON.stringify({ id: "fixture", object: "chat.completion.chunk", choices: [{ index: 0, delta, finish_reason: null }] })}\n\n`);
      res.end(`data: ${JSON.stringify({ id: "fixture", object: "chat.completion.chunk", choices: [{ index: 0, delta: {}, finish_reason: done ? "stop" : "tool_calls" }] })}\n\ndata: [DONE]\n\n`);
    } else {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ id: "fixture", object: "chat.completion", choices: [{ index: 0, message: delta, finish_reason: done ? "stop" : "tool_calls" }] }));
    }
  });
  await new Promise((accept) => server.listen(0, "127.0.0.1", accept));
  const port = server.address().port;
  const hookCommand = adapter
    ? `cmd.exe /d /s /c ""${resolve(adapter)}" --codecraft-kimi-hook"`
    : `"${process.execPath}" "${script}" --capture "${inbox}"`;
  const config = `default_model = "fixture"\ntelemetry = false\n[providers.fixture]\ntype = "openai"\nbase_url = "http://127.0.0.1:${port}/v1"\napi_key = "fixture-local"\n[models.fixture]\nprovider = "fixture"\nmodel = "fixture"\nmax_context_size = 32000\ncapabilities = ["tool_use"]\n[thinking]\nenabled = false\n[loop_control]\nmax_steps_per_turn = 6\nmax_attempts_per_step = 1\n`;
  await writeFile(join(root, "config.toml"), config + events.map((event) => `\n[[hooks]]\nevent = ${JSON.stringify(event)}\ncommand = ${JSON.stringify(hookCommand)}\ntimeout = 5\n`).join(""));
  const version = spawnSync(kimi, ["--version"], { encoding: "utf8", windowsHide: true }).stdout.trim();
  // Prompt mode auto-approves plans, so ACP supplies the real permission
  // service for this isolated protocol check. The integration remains Hook-only.
  const useAcp = scenario === "plan";
  const child = spawn(kimi, useAcp ? ["acp"] : ["-p", prompt], {
    cwd: root, windowsHide: true,
    env: { ...process.env, KIMI_CODE_HOME: root, LOCALAPPDATA: root }, stdio: [useAcp ? "pipe" : "ignore", "pipe", "pipe"],
  });
  let output = "";
  for (const stream of [child.stdout, child.stderr]) stream.on("data", (chunk) => { output = (output + chunk.toString()).slice(-8000); });
  const timeout = setTimeout(() => child.kill(), 45000);
  let acpError;
  const acpRun = useAcp ? runAcpPlan(child, root, prompt).catch((error) => { acpError = error; child.kill(); }) : Promise.resolve();
  const exitCode = await new Promise((accept, reject) => { child.on("error", reject); child.on("close", accept); });
  await acpRun;
  clearTimeout(timeout);
  server.closeAllConnections();
  await new Promise((accept) => server.close(accept));
  const payloads = await Promise.all((await readdir(inbox)).sort().map(async (file) => JSON.parse(await readFile(join(inbox, file), "utf8"))));
  await writeFile(join(root, "report.json"), JSON.stringify({ version, scenario, exitCode, requests, modelError, output, payloads }, null, 2));
  if (acpError) throw acpError;
  if (modelError) throw new Error(`${modelError}; see ${join(root, "report.json")}`);
  if (adapter) {
    const events = payloads.map((envelope) => envelope.payload);
    if (exitCode !== 0 || !events.some((event) => event.hook_event_name === "PostToolUse" || (scenario === "question" && event.hook_event_name === "PostToolUseFailure"))) throw new Error(`Production hook did not capture a complete tool run; see ${join(root, "report.json")}`);
    const identities = new Set(events.map((event) => `${event.pid}:${event.process_created_at}`));
    if (identities.size !== 1 || events.some((event) => !event.pid || !event.process_created_at)) throw new Error("Hook process identity is not stable");
    if (scenario !== "question" && !events.some((event) => event.tool_output)) throw new Error("Tool output was lost");
    const expectedTool = scenario === "question" ? "AskUserQuestion" : scenario === "plan" ? "ExitPlanMode" : "Read";
    if (!events.some((event) => event.hook_event_name === "PreToolUse" && event.tool_name === expectedTool)) throw new Error(`Missing ${expectedTool} capture; see ${join(root, "report.json")}`);
    if (scenario === "question" && !events.some((event) => event.tool_input?.questions?.[0]?.question === questions[0].question)) throw new Error("Question content was lost");
    if (scenario === "plan" && !events.some((event) => event.display?.kind === "plan_review" && event.display.plan === plan)) throw new Error(`Plan display was not captured; see ${join(root, "report.json")}`);
    for (const input of ['{}{}', 'x'.repeat(1024 * 1024 + 1)]) {
      const fallback = spawnSync(resolve(adapter), ['--codecraft-kimi-hook'], {
        input, encoding: 'utf8', windowsHide: true, timeout: 5000,
        env: { ...process.env, LOCALAPPDATA: root },
      });
      if (fallback.status !== 0 || fallback.stdout !== '' || !fallback.stderr) throw new Error('Invalid input changed native fallback behavior');
    }
  }
  console.log(JSON.stringify({ version, scenario, root, exitCode, requests,
    events: payloads.map((envelope) => { const event = adapter ? envelope.payload : envelope; return { event: event.hook_event_name, tool: event.tool_name, display: event.display?.kind }; }),
  }, null, 2));
}
