// Product CLI smoke only; these inputs are synthetic, not Cursor P0 evidence.
// Run from PowerShell after dot-sourcing limit-cpu.ps1.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { existsSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const directory = dirname(fileURLToPath(import.meta.url));
const executable = resolve(process.argv[2] ?? resolve(directory, '../../src-tauri/target/debug/codecraft-tauri.exe'));
assert.ok(existsSync(executable), 'Build the product executable first');
const isolated = resolve(directory, 'runs', `cli-smoke-${randomUUID()}`);
const env = { ...process.env, LOCALAPPDATA: isolated, USERPROFILE: isolated };
const invoke = (args, input) => {
  const result = spawnSync(executable, args, {
    input, env, encoding: 'utf8', windowsHide: true, timeout: 10000, maxBuffer: 1024 * 1024,
  });
  assert.ifError(result.error);
  assert.equal(result.signal, null);
  return result;
};
const cases = [
  ['preToolUse', '{malformed', { permission: 'allow' }],
  ['beforeSubmitPrompt', '{malformed', { continue: true }],
  ['beforeMCPExecution', '{}', { permission: 'allow' }],
  ['subagentStart', '{}', { permission: 'allow' }],
  ['afterAgentResponse', '{"conversation_id":"synthetic","text":"offline"}', {}],
  ['stop', '{}', {}],
];
for (const [event, input, expected] of cases) {
  const result = invoke(['--codecraft-cursor-hook', '--event', event], input);
  assert.equal(result.status, 0, `${event}: ${result.stderr}`);
  assert.deepEqual(JSON.parse(result.stdout.trim()), expected, event);
  console.log(`PASS ${event}: legal passive response without a broker`);
}
const mcp = invoke(['--codecraft-cursor-mcp'], '');
assert.equal(mcp.status, 1);
assert.equal(mcp.stdout, '');
assert.match(mcp.stderr, /INTEGRATION_DISABLED/);
assert.equal(existsSync(isolated), false, 'CLI modes must not create a profile or launch the GUI');
console.log('PASS disabled MCP exits without tools or GUI startup');
console.log(`7 product CLI checks passed: ${executable}`);
