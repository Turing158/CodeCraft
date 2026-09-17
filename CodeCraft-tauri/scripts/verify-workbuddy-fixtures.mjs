import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const project = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const root = path.join(project, "protocol/workbuddy/5.5.3");
const files = (await fs.readdir(path.join(root, "fixtures/runtime"))).filter((name) => name.endsWith(".runtime.json"));
const fixtures = new Map(await Promise.all(files.map(async (name) => [name, JSON.parse(await fs.readFile(path.join(root, "fixtures/runtime", name), "utf8"))])));
const fixture = (scenario, transport = "print") => fixtures.get(`${scenario}.${transport}.runtime.json`);

test("runtime fixtures are complete, versioned and distinct from synthetic examples", () => {
  assert.equal(fixtures.size, 13);
  const hashes = new Set();
  for (const value of fixtures.values()) {
    assert.equal(value.evidenceKind, "real-bundled-cli-with-deterministic-local-model");
    assert.equal(value.desktopUiValidated, false);
    assert.equal(value.protocolFrozen, false);
    assert.equal(value.result.promptCompleted, true);
    assert.equal(value.result.verifierTimedOut, false);
    assert.deepEqual(value.provenance.cliVersions, ["2.137.1"]);
    assert.match(value.provenance.bundleSha256, /^[0-9a-f]{64}$/);
    hashes.add(value.provenance.bundleSha256);
    const serialized = JSON.stringify(value);
    assert.ok(!serialized.includes("EducationalData"));
    assert.ok(!serialized.includes("Turing_ICE"));
    assert.ok(!serialized.includes('"messages":'));
    assert.ok(!serialized.includes('"stderr":'));
  }
  assert.equal(hashes.size, 1);
});

test("allow and deny outcomes reflect the actual sandbox file side effect", () => {
  for (const transport of ["print", "acp"]) {
    assert.ok(fixture("tool-allow", transport).result.workspaceFiles.includes("allowed.txt"));
    assert.equal(fixture("tool-allow", transport).nativeRequests.length, 0);
  }
  assert.ok(!fixture("tool-deny").result.workspaceFiles.includes("denied.txt"));
});

test("ask is negative evidence, not proof of waiting for a user", () => {
  assert.ok(fixture("tool-ask").result.workspaceFiles.includes("ask.txt"));
  assert.equal(fixture("tool-ask").hooks.filter((hook) => hook.input.hook_event_name === "PreToolUse").length, 2);
});

test("questions require native authorization before an answer-capable Hook", () => {
  for (const scenario of ["question-single", "question-multiple", "question-text"]) {
    const value = fixture(scenario, "acp");
    assert.equal(value.nativeRequests[0].method, "session/request_permission");
    assert.equal(value.nativeRequests[0].toolName, "AskUserQuestion");
    assert.ok(value.hooks.every((hook) => !["PreToolUse", "PermissionRequest"].includes(hook.input.hook_event_name)));
  }
});

test("plan inputs are schema-valid but cannot identify the reviewed body", () => {
  for (const scenario of ["plan-allow", "plan-deny"]) {
    const value = fixture(scenario, "acp");
    assert.deepEqual(value.policy.args, { allowedPrompts: [] });
    assert.ok(value.hooks.filter((hook) => hook.input.tool_name === "ExitPlanMode")
      .every((hook) => !Object.hasOwn(hook.input.tool_input, "plan")));
  }
  const result = fixture("plan-allow", "acp").hooks.find((hook) => hook.input.tool_name === "ExitPlanMode" && hook.input.hook_event_name === "PostToolUse");
  assert.equal(result.input.tool_response.plan, "");
});

test("timeout, empty fallback and tool failures remain negative outcomes", () => {
  assert.ok(!fixture("timeout").result.workspaceFiles.includes("timeout.txt"));
  assert.ok(fixture("timeout").toolResults.some((result) => result.text.includes("timed out after 1000ms")));
  assert.ok(!fixture("fallback").result.workspaceFiles.includes("fallback.txt"));
  assert.ok(fixture("fallback").hooks.some((hook) => hook.input.hook_event_name === "PermissionDenied"));
  const failure = fixture("failure").hooks.find((hook) => hook.input.hook_event_name === "PostToolUse");
  assert.equal(failure.input.tool_response.is_error, true);
});

test("the released capability matrix continues to reject all writes", async () => {
  const capabilities = JSON.parse(await fs.readFile(path.join(root, "capabilities.json"), "utf8"));
  for (const key of ["canApproveTools", "canAnswerQuestions", "canApprovePlans", "canStreamOutput"]) assert.equal(capabilities[key].value, false);
});
