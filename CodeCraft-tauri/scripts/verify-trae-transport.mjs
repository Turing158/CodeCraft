import { spawn, spawnSync } from 'node:child_process';
import { connect } from 'node:net';
import { mkdir, readFile, writeFile, rename, rm } from 'node:fs/promises';
import { resolve, join, sep } from 'node:path';
import { randomUUID } from 'node:crypto';
import assert from 'node:assert/strict';

const base = resolve('src-tauri/trae-core/target/transport-tests');
const root = join(base, randomUUID());
const executable = resolve('src-tauri/trae-core/target/debug/examples/bridge_test_host.exe');
const option = name => { const i = process.argv.indexOf(name); return i < 0 ? null : process.argv[i + 1]; };
const sandboxExe = option('--sandbox-exe');
const sandboxStorage = option('--sandbox-storage');
const sandboxConfig = option('--sandbox-config');
const bundled = process.argv.includes('--bundled');
await mkdir(root, { recursive: true });
const store = spawn(executable, [bundled ? 'store-bundled' : 'store', root], { windowsHide: true, stdio: ['pipe', 'ignore', 'pipe'] });
let storeErrors = '';
store.stderr.on('data', data => storeErrors += data);
const read = async path => { try { return JSON.parse(await readFile(path, 'utf8')); } catch { return null; } };
async function until(fn, ms = 10000) {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) { const value = await fn(); if (value) return value; await new Promise(r => setTimeout(r, 30)); }
  throw new Error(`Transport test timed out: ${storeErrors}`);
}
let endpoint;
function request(command, extra = {}) {
  return { schemaVersion: 1, appEpoch: endpoint.appEpoch, token: endpoint.token, commandId: randomUUID(), command, ...extra };
}
function frame(value) {
  const body = Buffer.from(typeof value === 'string' ? value : JSON.stringify(value));
  const header = Buffer.alloc(4); header.writeUInt32BE(body.length);
  return Buffer.concat([header, body]);
}
function exchange(bytes) {
  return new Promise((resolveReply, reject) => {
    const socket = connect({ host: '127.0.0.1', port: endpoint.port });
    let received = Buffer.alloc(0), done = false;
    const finish = value => { if (!done) { done = true; socket.destroy(); resolveReply(value); } };
    socket.setTimeout(8000, () => socket.destroy(new Error('Transport response timed out')));
    socket.on('connect', () => socket.write(bytes));
    socket.on('data', chunk => {
      received = Buffer.concat([received, chunk]);
      if (received.length >= 4 && received.length >= 4 + received.readUInt32BE(0)) {
        try { finish(JSON.parse(received.subarray(4, 4 + received.readUInt32BE(0)).toString())); } catch (e) { reject(e); socket.destroy(); }
      }
    });
    socket.on('end', () => finish(null));
    socket.on('error', e => { if (!done) reject(e); });
  });
}
const network = message => exchange(frame(message));
function hook(agent, session, event = 'UserPromptSubmit', extra = {}) {
  return { kind: 'hook', identity: { installationId: 'synthetic', traeInstanceId: 'transport-test' }, input: {
    hook_event_name: event, session_id: session, agent_id: agent, agent_type: agent,
    cwd: '.', workspace_roots: [], prompt: `Synthetic ${agent} transport test`, ...extra,
  } };
}
async function desktop(command) {
  const id = randomUUID(), path = join(root, 'inbox', `${id}.json`);
  await writeFile(path + '.tmp', JSON.stringify({ schemaVersion: 1, appEpoch: endpoint.appEpoch, messageId: randomUUID(), commandId: id, command }));
  await rename(path + '.tmp', path);
  return until(() => read(join(root, 'replies', `${id}.json`)));
}
let passed = 0, sandboxScenarios = 0;
try {
  endpoint = await until(async () => (await read(join(root, 'heartbeat.json'))) && read(join(root, 'hook-endpoint.json')));
  const first = request(hook('chat', 'chat-local'));
  assert.deepEqual((await network(first)).result.output, {});
  assert.deepEqual((await network(first)).result.output, {});
  const state = await read(join(root, 'state.json'));
  if (bundled) {
    assert.equal(state.capabilities.toolApproval, true);
    assert.equal(state.capabilities.mcpQuestions, false);
    assert.equal(state.capabilities.mcpPlanReview, false);
    assert.equal(state.capabilities.verifiedVersion, null);
  }
  assert.equal(Object.values(state.sessions).filter(s => s.sessionId === 'chat-local').length, 1); passed++;
  const changed = structuredClone(first); changed.command.input.prompt = 'changed';
  assert.equal((await network(changed)).error.error.code, 'IDEMPOTENCY_CONFLICT'); passed++;
  assert.equal((await network(request(hook('chat', 'bad-token'), { token: 'wrong' }))).error.error.code, 'FORBIDDEN'); passed++;
  assert.equal((await network(request(hook('chat', 'old-epoch'), { appEpoch: randomUUID() }))).error.error.code, 'REQUEST_EXPIRED'); passed++;
  for (const kind of ['decision', 'create_task', 'refresh_capabilities'])
    assert.equal((await network(request({ kind }))).error.error.code, 'FORBIDDEN');
  passed++;
  const huge = Buffer.alloc(4); huge.writeUInt32BE(2 * 1024 * 1024 + 1);
  assert.equal(await exchange(huge), null);
  assert.equal(await exchange(frame('{"token":"a","token":"b"}')), null); passed++;
  assert.deepEqual((await network(request(hook('solo_agent', 'solo-local')))).result.output, {}); passed++;

  const tool = hook('solo_agent', 'tool-local', 'PreToolUse', { cwd: root, workspace_roots: [root], tool_use_id: 'synthetic-tool', tool_name: 'RunCommand', llm_tool_name: 'RunCommand', tool_input: { command: 'echo synthetic' } });
  const registered = await network(request(tool)); assert.equal(registered.error, null);
  const operation = registered.result.requestId;
  const pending = (await read(join(root, 'state.json'))).requests[operation];
  if (pending.state === 'pending') {
    const approved = await desktop({ kind: 'decision', body: { schemaVersion: 1, decisionId: randomUUID(), target: pending.target, action: { kind: 'permission', decision: 'allow', message: null } } });
    assert.equal(approved.error, null);
  }
  assert.equal((await network(request({ kind: 'poll', operation }))).result.state, 'user_decided');
  const prepared = await network(request({ kind: 'prepare', operation }));
  assert.equal(prepared.result.output.hookSpecificOutput.permissionDecision, 'allow');
  assert.equal((await network(request({ kind: 'ack', operation, lease: prepared.result.lease }))).error, null); passed++;

  if (sandboxExe) {
    assert.ok(sandboxStorage && sandboxConfig, 'Supply the existing sandbox storage and config name');
    await mkdir(join(root, 'sandbox-logs'), { recursive: true });
    await mkdir(join(root, 'sandbox-dumps'), { recursive: true });
    const quote = value => `'${value.replaceAll("'", "''")}'`;
    for (const agent of ['chat', 'solo_agent']) {
      const command = hook(agent, `sandbox-${agent}`);
      const result = spawnSync(sandboxExe, ['exec', '--storage-path', sandboxStorage, '--config-name', sandboxConfig,
        '--shell-path', 'powershell.exe', '--command-line', `& ${quote(executable)} transport ${quote(root)}`],
        { input: JSON.stringify(command) + '\n', encoding: 'utf8', windowsHide: true, timeout: 15000,
          env: { ...process.env, TRAE_SANDBOX_CLI_PATH: sandboxExe, TRAE_SANDBOX_LOG_DIR: join(root, 'sandbox-logs'), TRAE_SANDBOX_DUMP_DIR: join(root, 'sandbox-dumps') } });
      assert.ifError(result.error);
      assert.equal(result.status, 0, result.stderr);
      const reply = JSON.parse(result.stdout.trim());
      assert.equal(reply.error, null); assert.deepEqual(reply.result.output, {});
      const snapshot = await read(join(root, 'state.json'));
      const session = Object.values(snapshot.sessions).find(s => s.sessionId === `sandbox-${agent}`);
      assert.equal(session.title, command.input.prompt); assert.equal(session.status, 'working');
      sandboxScenarios++; passed++;
    }
  }
  console.log(JSON.stringify({ suite: 'Trae authenticated loopback transport', passed, sandboxScenarios, bundledCapabilities: bundled,
    sandboxConfigurationModified: false, realTraeAgentConversationVerified: false }));
} finally {
  store.stdin.end();
  try { await until(() => store.exitCode !== null, 4000); } catch { store.kill(); }
  await new Promise(r => setTimeout(r, 200));
  assert.ok(root.startsWith(base + sep));
  await rm(root, { recursive: true, force: true });
}
