// Replays a redacted real Chat payload through the production Hook executable.
// No Trae process, bridge connection or tool execution is simulated as verified.
import { spawnSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import assert from 'node:assert/strict';

const executable = resolve(process.argv[2] ?? 'src-tauri/target/debug/codecraft-tauri.exe');
const fixture = JSON.parse(await readFile('protocol/trae/3.3.102/fixtures/workspace-less-chat.json', 'utf8'));
let passed = 0;
function replay(event, input) {
  const result = spawnSync(executable, ['--codecraft-trae-hook', '--event', event], {
    input: JSON.stringify(input), encoding: 'utf8', windowsHide: true, timeout: 10000,
  });
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
  return JSON.parse(result.stdout);
}
for (const agent of ['chat', 'solo_agent']) {
for (const event of ['UserPromptSubmit', 'SessionStart', 'Notification', 'Stop']) {
  const input = { ...fixture.input, agent_type: agent, agent_id: agent, hook_event_name: event };
  if (event === 'Notification') Object.assign(input, { notification_type: 'idle_prompt', message: 'Agent has completed the task' });
  assert.deepEqual(replay(event, input), fixture.expectedOutputAfterFix);
  passed++;
}
}
for (const tool of ['RunCommand', 'mcp__codecraft_probe__codecraft_probe_echo']) {
  const input = { ...fixture.input, hook_event_name: 'PreToolUse', tool_use_id: 'unscoped-tool', tool_name: tool, llm_tool_name: tool, tool_input: { payload: 'test' } };
  assert.equal(replay('PreToolUse', input).hookSpecificOutput.permissionDecision, 'deny');
  passed++;
}
for (const extra of [{ prompt: null }, { hook_event_name: 'PreToolUse' }]) {
  assert.equal(replay('UserPromptSubmit', { ...fixture.input, ...extra }).decision, 'block');
  passed++;
}
console.log(JSON.stringify({ suite: 'Trae standalone Chat production Hook replay', passed, inputEvidence: fixture.evidenceKind, fullTraeToolApprovalRuntimeVerified: false }));
