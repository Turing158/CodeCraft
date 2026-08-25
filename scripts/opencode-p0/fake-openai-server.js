import { createServer } from "node:http";

const port = Number.parseInt(process.argv[2] ?? "48769", 10);
let callSequence = 0;

function chunk(delta = {}, finishReason) {
  return {
    id: "chatcmpl-codecraft-p0",
    object: "chat.completion.chunk",
    created: Math.floor(Date.now() / 1000),
    model: "probe-model",
    choices: [{ index: 0, delta, ...(finishReason ? { finish_reason: finishReason } : {}) }],
  };
}

function sendStream(response, chunks) {
  response.writeHead(200, {
    "content-type": "text/event-stream",
    "cache-control": "no-cache",
    connection: "keep-alive",
  });
  for (const item of chunks) response.write(`data: ${JSON.stringify(item)}\n\n`);
  response.end("data: [DONE]\n\n");
}

function questionInput() {
  return {
    questions: [
      {
        question: "Approve the isolated CodeCraft HTTP probe?",
        header: "HTTP probe",
        options: [
          { label: "Yes", description: "Continue the isolated probe." },
          { label: "No", description: "Reject the isolated probe." },
        ],
        multiple: false,
        custom: false,
      },
    ],
  };
}

function toolStream(name, input) {
  callSequence += 1;
  const id = `call_codecraft_${callSequence}`;
  return [
    chunk({ role: "assistant" }),
    chunk({
      tool_calls: [
        { index: 0, id, type: "function", function: { name, arguments: "" } },
      ],
    }),
    chunk({ tool_calls: [{ index: 0, function: { arguments: JSON.stringify(input) } }] }),
    chunk({}, "tool_calls"),
  ];
}

const server = createServer((request, response) => {
  if (request.method === "GET" && request.url === "/health") {
    response.writeHead(200, { "content-type": "application/json" });
    response.end('{"ok":true}');
    return;
  }
  if (request.method !== "POST" || request.url !== "/v1/chat/completions") {
    response.writeHead(404, { "content-type": "application/json" });
    response.end('{"error":"not found"}');
    return;
  }

  let raw = "";
  request.setEncoding("utf8");
  request.on("data", (part) => {
    raw += part;
  });
  request.on("end", () => {
    const lower = raw.toLowerCase();
    const toolResults = (raw.match(/"role":"tool"/g) ?? []).length;
    if (raw.includes("Generate a title for this conversation")) {
      sendStream(response, [chunk({ role: "assistant" }), chunk({ content: "CodeCraft P0 Probe" }), chunk({}, "stop")]);
      return;
    }
    if (raw.includes("PERMISSION_ALWAYS") && toolResults < 2) {
      sendStream(
        response,
        toolStream("bash", { command: "Write-Output codecraft-http-probe", timeout: 5000 }),
      );
      return;
    }
    if (lower.includes('"role":"tool"') || lower.includes('"tool_call_id"')) {
      sendStream(response, [chunk({ role: "assistant" }), chunk({ content: "Probe completed." }), chunk({}, "stop")]);
      return;
    }
    if (raw.includes("QUESTION_")) {
      sendStream(response, toolStream("question", questionInput()));
      return;
    }
    if (raw.includes("PERMISSION_")) {
      sendStream(
        response,
        toolStream("bash", { command: "Write-Output codecraft-http-probe", timeout: 5000 }),
      );
      return;
    }
    sendStream(response, [chunk({ role: "assistant" }), chunk({ content: "Probe ready." }), chunk({}, "stop")]);
  });
});

server.listen(port, "127.0.0.1", () => {
  process.stdout.write(`fake OpenAI server listening on http://127.0.0.1:${port}/v1\n`);
});

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => server.close(() => process.exit(0)));
}
