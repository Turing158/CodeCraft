// Invoked only by the isolated real-runtime protocol capture harness.
import fs from "node:fs/promises";
import path from "node:path";
import crypto from "node:crypto";

const chunks = [];
let length = 0;
for await (const chunk of process.stdin) {
  length += chunk.length;
  if (length > 256 * 1024) throw new Error("capture input too large");
  chunks.push(chunk);
}
const input = JSON.parse(Buffer.concat(chunks).toString("utf8"));
const directory = process.env.CODECRAFT_CAPTURE_DIR;
if (!directory) throw new Error("isolated capture directory required");
const policy = JSON.parse(await fs.readFile(path.join(directory, "policy.json"), "utf8"));
let response = {};
if (input.hook_event_name === "PreToolUse") {
  if (policy.delayMs) await new Promise((resolve) => setTimeout(resolve, policy.delayMs));
  if (policy.decision) response = {
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      permissionDecision: policy.decision,
      permissionDecisionReason: "CodeCraft isolated protocol verification",
      ...(policy.modifiedInput ? { modifiedInput: policy.modifiedInput } : {}),
    },
  };
}
await fs.writeFile(path.join(directory, `${Date.now()}-${crypto.randomUUID()}.hook.json`), JSON.stringify({input, response}, null, 2), {flag:"wx"});
process.stdout.write(JSON.stringify(response) + "\n");
