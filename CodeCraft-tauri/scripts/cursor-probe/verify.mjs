import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import test from 'node:test';
import { inspectClient } from './inspect-client.mjs';

const directory = dirname(fileURLToPath(import.meta.url));
const executable = join(directory, 'target', 'debug', `codecraft-cursor-probe${process.platform === 'win32' ? '.exe' : ''}`);
const runs = join(directory, 'runs');
mkdirSync(runs, { recursive: true });
function run(args, input = '') {
  const result = spawnSync(executable, args, { input, encoding: 'utf8', timeout: 10_000, windowsHide: true });
  assert.ifError(result.error);
  return result;
}
function fixture() {
  const parent = mkdtempSync(join(runs, 'synthetic-'));
  const root = join(parent, 'run with spaces');
  assert.equal(run(['init', '--root', root]).status, 0);
  const manifest = JSON.parse(readFileSync(join(root, 'run.json'), 'utf8'));
  return { root, manifest, project: manifest.project };
}
function event(f, name, extra = {}) {
  return { hook_event_name: name, workspace_roots: [f.project], cwd: f.project, conversation_id: 'synthetic-session', generation_id: 'synthetic-generation', ...extra };
}
function captures(root) {
  return readdirSync(join(root, 'events')).filter(name => name.endsWith('.json')).map(name => JSON.parse(readFileSync(join(root, 'events', name), 'utf8')));
}

test('synthetic: passive Hook responses remain valid on malformed input and disk failure', () => {
  for (const name of ['preToolUse', 'beforeMCPExecution', 'subagentStart']) {
    const response = run(['hook', '--root', join(runs, 'missing'), '--event', name], '{bad');
    assert.equal(response.status, 0);
    assert.deepEqual(JSON.parse(response.stdout), { permission: 'allow' });
  }
  const unknown = run(['hook', '--root', runs, '--event', 'Unknown'], '{}');
  assert.equal(unknown.status, 1);
  assert.equal(unknown.stdout, '');
});

test('synthetic: ambiguous MCP names never inject or change third-party inputs', () => {
  const f = fixture();
  const input = event(f, 'preToolUse', { tool_name: 'MCP:codecraft_probe_echo', tool_input: { schemaVersion: 1, clientRequestId: 'a', payload: {}, bridgeTicket: 'model-supplied-secret' } });
  const response = run(['hook', '--root', f.root, '--event', 'preToolUse'], JSON.stringify(input));
  assert.deepEqual(JSON.parse(response.stdout), { permission: 'allow' });
  const events = captures(f.root);
  assert.equal(events.length, 1);
  assert.equal(events[0].details.identityAssessment, 'UNQUALIFIED_MCP_IDENTITY');
  assert.equal(events[0].details.inputModified, false);
  assert.ok(!JSON.stringify(events).includes('model-supplied-secret'));
  assert.equal(events[0].cursorClientExecuted, null);
});

test('synthetic: workspace boundaries and malformed inputs cannot become captured Hook events', () => {
  const f = fixture();
  const inputs = [
    event(f, 'sessionStart', { workspace_roots: [f.root] }),
    event(f, 'sessionStart', { workspace_roots: [f.project, f.root] }),
    event(f, 'sessionStart'),
  ];
  for (const input of inputs) run(['hook', '--root', f.root, '--event', 'preToolUse'], JSON.stringify(input));
  run(['hook', '--root', f.root, '--event', 'preToolUse'], '{"tool_input":{"x":1,"x":2}}');
  const records = captures(f.root);
  assert.equal(records.length, 1);
  assert.equal(records[0].event, 'inputRejected');
  assert.equal(records[0].input, null);
});

test('synthetic: observed Windows BOM and slash-drive roots retain strict project isolation', { skip: process.platform !== 'win32' }, () => {
  const f = fixture();
  const ordinaryPath = f.project.startsWith('\\\\?\\') ? f.project.slice(4) : f.project;
  const uriPath = '/' + ordinaryPath.replaceAll('\\', '/');
  const input = event(f, 'preToolUse', { workspace_roots: [uriPath], cwd: uriPath, tool_name: 'Read', tool_input: { path: 'sample.txt' } });
  const result = run(['hook', '--root', f.root, '--event', 'preToolUse'], '\ufeff' + JSON.stringify(input));
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(JSON.parse(result.stdout), { permission: 'allow' });
  const records = captures(f.root);
  assert.equal(records.length, 1);
  assert.equal(records[0].event, 'preToolUse');
  assert.equal(records[0].details.utf8BomRemoved, true);
  const outside = event(f, 'preToolUse', { workspace_roots: [uriPath + '/..'], tool_name: 'Read' });
  run(['hook', '--root', f.root, '--event', 'preToolUse'], '\ufeff' + JSON.stringify(outside));
  assert.equal(captures(f.root).length, 1);
});

test('synthetic: concurrent Hook processes publish unique complete events and matching aliases', async () => {
  const f = fixture();
  const input = JSON.stringify(event(f, 'postToolUseFailure', { tool_use_id: 'same-native-call', tool_input: { bridgeTicket: 'same-secret' }, failure_type: 'permission_denied' }));
  await Promise.all(Array.from({ length: 2 }, () => new Promise((resolve, reject) => {
    const child = spawn(executable, ['hook', '--root', f.root, '--event', 'postToolUseFailure'], { windowsHide: true });
    let stdout = '';
    child.stdout.on('data', data => { stdout += data; });
    child.on('error', reject);
    child.on('exit', code => {
      try { assert.equal(code, 0); assert.deepEqual(JSON.parse(stdout), {}); resolve(); } catch (error) { reject(error); }
    });
    child.stdin.end(input);
  })));
  const records = captures(f.root);
  assert.deepEqual(records.map(r => r.sequence), [1, 2]);
  assert.notEqual(records[0].eventId, records[1].eventId);
  assert.equal(records[0].ticketAlias, records[1].ticketAlias);
  assert.ok(!JSON.stringify(records).includes('same-secret'));
});

test('synthetic: actual stdio process accepts ID zero, isolates server identity, rejects RPC replay', () => {
  const f = fixture();
  const frames = [
    { jsonrpc: '2.0', id: 0, method: 'initialize', params: { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'synthetic', version: '1' } } },
    { jsonrpc: '2.0', method: 'notifications/initialized' },
    { jsonrpc: '2.0', id: 1, method: 'tools/list' },
    { jsonrpc: '2.0', id: 2, method: 'tools/call', params: { name: 'codecraft_probe_echo', arguments: { schemaVersion: 1, clientRequestId: 'test', payload: { message: 'hello' }, bridgeTicket: 'secret' } } },
  ];
  frames.push(frames[3], { jsonrpc: '2.0', id: 2, method: 'ping' });
  const response = run(['mcp', '--root', f.root, '--server', 'collision'], frames.map(f => JSON.stringify(f)).join('\n') + '\n');
  assert.equal(response.status, 0, response.stderr);
  const output = response.stdout.trim().split('\n').map(line => JSON.parse(line));
  assert.equal(output.length, 4);
  assert.equal(output[0].id, 0);
  assert.ok(!output[1].result.tools[0].inputSchema.required.includes('bridgeTicket'));
  const result = JSON.parse(output[2].result.content[0].text);
  assert.equal(result.server, 'collision');
  assert.equal(result.workflowEnabled, false);
  assert.equal(output[3].error.message, 'RPC_ID_CONFLICT');
  assert.ok(!JSON.stringify(captures(f.root)).includes('"secret"'));
});

test('synthetic: malformed and oversized stdio frames stop without unbounded allocation', () => {
  const f = fixture();
  for (const text of ['{"a":1,"a":2}\n', ' '.repeat(1024 * 1024 + 1)]) {
    const result = run(['mcp', '--root', f.root, '--server', 'owned'], text);
    assert.equal(result.status, 1);
    assert.equal(JSON.parse(result.stdout).error.code, -32700);
  }
});

test('synthetic: passed labels cannot bypass evidence, version or native capability checks', () => {
  const f = fixture();
  const runtime = { cursorVersion: '3.21.13', mode: 'Agent', platform: 'windows', architecture: 'x64', clientFingerprint: 'client', configFingerprint: 'config' };
  writeFileSync(join(f.root, 'runtime.json'), JSON.stringify(runtime));
  const checks = Object.fromEntries(Array.from({ length: 11 }, (_, i) => [`P0-${String(i + 1).padStart(2, '0')}`, { status: 'passed', verifiedAt: '2026-09-21', reason: '', evidence: [] }]));
  writeFileSync(join(f.root, 'compatibility.json'), JSON.stringify({ schemaVersion: 1, identity: runtime, captureScope: 'observedOnly', checks, toolCoverage: {}, qualifiedToolNames: {}, protocol: {}, nativeAdapter: {} }));
  const result = run(['compatibility', '--record', join(f.root, 'compatibility.json'), '--runtime', join(f.root, 'runtime.json'), '--tool', 'Shell']);
  assert.equal(result.status, 0, result.stderr);
  const capabilities = JSON.parse(result.stdout);
  assert.ok(capabilities.every(c => !c.supported));
  assert.ok(capabilities.find(c => c.capability === 'nativeShared').reasons.includes('V27_NOT_VERIFIED'));
});

test('synthetic: a reviewed evidence fixture is bound to its case, mode and content hash', () => {
  // These fabricated fixtures only exercise the validator, live in ignored
  // synthetic runs, and are never exported as Cursor compatibility evidence.
  const f = fixture();
  const runtime = { cursorVersion: 'fixture-version', mode: 'Agent', platform: 'windows', architecture: 'x64', clientFingerprint: 'fixture-client', configFingerprint: 'fixture-config' };
  const required = ['P0-01', 'P0-03', 'P0-05', 'P0-07', 'P0-08', 'tool:Shell'];
  const evidence = { evidenceKind: 'cursorRuntime', cursorClientExecuted: true, probeInjected: false, identity: runtime, checkIds: required, checkResults: Object.fromEntries(required.map(id => [id, 'passed'])) };
  const bytes = JSON.stringify(evidence);
  writeFileSync(join(f.root, 'fixture.json'), bytes);
  const check = { status: 'passed', verifiedAt: 'synthetic-date', reason: 'Validator test only', evidence: [{ path: 'fixture.json', kind: 'cursorRuntime', sha256: createHash('sha256').update(bytes).digest('hex') }] };
  const record = { schemaVersion: 1, identity: runtime, captureScope: 'observedOnly', checks: Object.fromEntries(required.slice(0, -1).map(id => [id, check])), toolCoverage: { Shell: check }, qualifiedToolNames: {}, protocol: {}, nativeAdapter: {} };
  writeFileSync(join(f.root, 'record.json'), JSON.stringify(record));
  const evaluate = current => {
    writeFileSync(join(f.root, 'runtime.json'), JSON.stringify(current));
    const result = run(['compatibility', '--record', join(f.root, 'record.json'), '--runtime', join(f.root, 'runtime.json'), '--tool', 'Shell']);
    assert.equal(result.status, 0, result.stderr);
    return JSON.parse(result.stdout);
  };
  assert.equal(evaluate(runtime)[0].supported, true);
  assert.equal(evaluate({ ...runtime, mode: 'Ask' })[0].supported, false);
  assert.equal(evaluate({ ...runtime, clientFingerprint: 'new-build' })[0].supported, false);
  for (const outcome of ['failed', 'notRun', undefined]) {
    evidence.checkResults['P0-03'] = outcome;
    const changed = JSON.stringify(evidence);
    writeFileSync(join(f.root, 'fixture.json'), changed);
    check.evidence[0].sha256 = createHash('sha256').update(changed).digest('hex');
    writeFileSync(join(f.root, 'record.json'), JSON.stringify(record));
    assert.equal(evaluate(runtime)[0].supported, false, `Cannot promote ${outcome} evidence to passed`);
  }
  evidence.checkResults['P0-03'] = 'passed';
  check.evidence[0].sha256 = createHash('sha256').update(bytes).digest('hex');
  writeFileSync(join(f.root, 'record.json'), JSON.stringify(record));
  writeFileSync(join(f.root, 'fixture.json'), bytes + ' ');
  assert.equal(evaluate(runtime)[0].supported, false);
  evidence.evidenceKind = 'synthetic';
  const synthetic = JSON.stringify(evidence);
  writeFileSync(join(f.root, 'fixture.json'), synthetic);
  check.evidence[0].sha256 = createHash('sha256').update(synthetic).digest('hex');
  writeFileSync(join(f.root, 'record.json'), JSON.stringify(record));
  assert.equal(evaluate(runtime)[0].supported, false);
});

test('synthetic: changed probe builds cannot attribute captures to an old executable hash', () => {
  const f = fixture();
  writeFileSync(join(f.root, 'run.json'), JSON.stringify({ ...f.manifest, probeSha256: 'old-build' }));
  const result = run(['hook', '--root', f.root, '--event', 'preToolUse'], JSON.stringify(event(f, 'preToolUse')));
  assert.equal(result.status, 0);
  assert.deepEqual(JSON.parse(result.stdout), { permission: 'allow' });
  assert.match(result.stderr, /PROBE_BUILD_CHANGED/);
  assert.equal(captures(f.root).length, 0);
});

test('reviewed runtime regression: identical MCP calls cannot establish candidate server identity', () => {
  const modeDirectory = join(directory, '../../protocol/cursor/3.21.16/Agent');
  const record = JSON.parse(readFileSync(join(modeDirectory, 'compatibility.json'), 'utf8'));
  const check = record.checks['P0-03'];
  assert.equal(check.status, 'failed');
  const reference = check.evidence[0];
  const bytes = readFileSync(join(modeDirectory, reference.path));
  assert.equal(createHash('sha256').update(bytes).digest('hex'), reference.sha256);
  const evidence = JSON.parse(bytes);
  assert.deepEqual(evidence.identity, record.identity);
  assert.equal(evidence.checkResults['P0-03'], 'failed');
  assert.equal(evidence.cursorClientExecuted, true);
  assert.equal(evidence.probeInjected, false);
  const at = sequence => evidence.captures.find(e => e.sequence === sequence);
  const trial = evidence.review.identicalArgumentsTurn;
  const [left, right] = trial.preToolUse.map(at);
  assert.equal(left.event, 'preToolUse');
  assert.equal(right.event, 'preToolUse');
  assert.equal(left.toolName, 'MCP:codecraft_probe_echo');
  assert.equal(right.toolName, left.toolName);
  assert.notDeepEqual(left.input.tool_use_id, right.input.tool_use_id);
  const withoutId = e => { const input = structuredClone(e.input); delete input.tool_use_id; return input; };
  assert.deepEqual(withoutId(left), withoutId(right));
  assert.equal(left.input.mcp_server_name, undefined);
  assert.equal(left.input.command, undefined);
  const before = trial.beforeMCPExecution.map(at);
  assert.deepEqual(before.map(e => e.probeServerName), ['codecraft_cursor_probe', 'codecraft_cursor_collision']);
  assert.ok(before.every(e => e.event === 'beforeMCPExecution' && e.input.tool_use_id === undefined));
  const calls = trial.toolsCall.map(at);
  assert.deepEqual(calls.map(e => e.details.server), ['owned', 'collision']);
  assert.deepEqual(calls[0].input.params.arguments, calls[1].input.params.arguments);
  assert.deepEqual(calls[0].input.params.arguments, left.parsedToolInput);
  for (const [i, sequence] of trial.postToolUse.entries()) {
    assert.deepEqual(at(sequence).input.tool_use_id, [left, right][i].input.tool_use_id);
  }
  assert.ok(evidence.captures.every(e => e.evidenceKind === 'probeCapture' && e.probeSha256 === evidence.probe.sha256));
  assert.ok(!bytes.toString('utf8').includes('aliasSalt'));
});

test('reviewed runtime regression: current version and untested modes keep every capability disabled', () => {
  const f = fixture();
  const runtimeRecord = JSON.parse(readFileSync(join(directory, '../../protocol/cursor/3.21.16/Agent/compatibility.json'), 'utf8'));
  // This is an offline validator regression, not an independently observed
  // production handshake or a new Cursor runtime experiment.
  for (const mode of ['Agent', 'Ask', 'Plan']) {
    writeFileSync(join(f.root, 'runtime.json'), JSON.stringify({ ...runtimeRecord.identity, mode }));
    const result = run(['compatibility', '--record', join(directory, `../../protocol/cursor/3.21.16/${mode}/compatibility.json`), '--runtime', join(f.root, 'runtime.json'), '--tool', 'codecraft_probe_echo']);
    assert.equal(result.status, 0, result.stderr);
    const capabilities = JSON.parse(result.stdout);
    assert.ok(capabilities.every(c => !c.supported));
    assert.ok(capabilities.every(c => c.reasons.includes('P0-03_NOT_VERIFIED')));
  }
});

function sourceFixture(files = {}) {
  const app = mkdtempSync(join(runs, 'synthetic-source-'));
  for (const [relative, text] of Object.entries({ 'package.json': '{"version":"synthetic"}', ...files })) {
    const path = join(app, relative);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, text);
  }
  return app;
}

test('source inspection: missing or changed bundles yield unknown conclusions, never old capabilities', () => {
  const app = sourceFixture();
  for (const files of [[], ['extensions/cursor-agent-host/dist/main.js', 'out/vs/workbench/api/node/extensionHostProcess.js']]) {
    for (const file of files) {
      mkdirSync(dirname(join(app, file)), { recursive: true });
      writeFileSync(join(app, file), '/* a future, unrecognized client layout */');
    }
    const result = inspectClient(app);
    for (const key of ['mcpCandidateIdentity', 'nativeApprovalRouting', 'extensionProposalAccess', 'runtimeAccess', 'interactionRegistry']) {
      assert.equal(result.conclusions[key], 'unknown');
    }
    assert.equal(result.conclusions.nativeShared, 'unverified');
    assert.equal(result.conclusions.supported, false);
    assert.equal(result.cursorClientExecuted, false);
    assert.equal(result.architecture, null);
  }
});

test('source inspection: observed internal APIs and builtin guards remain source-only leads', () => {
  const app = sourceFixture({
    'extensions/cursor-agent-host/dist/main.js': 'const h=`MCP:${t.toolName}`; pendingDecisionProvider.requestApproval(); interactionRegistry.respond(); interactionRegistry.watch();',
    'extensions/cursor-agent-exec/dist/main.js': 'async requestApproval(e){if(e.toolCallId) return this.approvalHandler(e)}',
    'out/vs/workbench/api/node/extensionHostProcess.js': [
      'function a(extension,proposal){return(proposal==="cursor"||proposal==="cursorAgentHost")&&!extension.isBuiltin}',
      'proposals are only available for built-in extensions',
      'function b(extension,id){if(a(extension,"cursorAgentHost"),!extension.isBuiltin||!Ids.equals(extension.identifier,id)) throw Error()}',
      'acquireAgentHostRuntime(){return b(extension,id,"acquireAgentHostRuntime")}',
      'this._proxy.$requestAgentExecApproval(session,request)',
    ].join('\n'),
    'out/vs/workbench/workbench.desktop.main.js': '$requestAgentExecApproval(session,request)',
  });
  const result = inspectClient(app);
  assert.equal(result.conclusions.mcpCandidateIdentity, 'unqualifiedWrapperObserved');
  assert.equal(result.conclusions.nativeApprovalRouting, 'internalCallbackChainObserved');
  assert.equal(result.conclusions.extensionProposalAccess, 'builtinOnlyGuardObserved');
  assert.equal(result.conclusions.runtimeAccess, 'builtinExtensionIdentityGuardObserved');
  assert.equal(result.conclusions.interactionRegistry, 'internalRespondAndWatchObserved');
  assert.equal(result.evidenceKind, 'sourceAnalysis');
  assert.equal(result.conclusions.supported, false);
  assert.equal(result.cursorClientExecuted, false);
  const path = join(app, 'out/vs/workbench/api/node/extensionHostProcess.js');
  writeFileSync(path, 'proposals are only available for built-in extensions');
  const changed = inspectClient(app);
  assert.equal(changed.conclusions.extensionProposalAccess, 'unknown', 'An error message alone is not a verified guard');
  assert.equal(changed.conclusions.runtimeAccess, 'unknown');
  assert.notEqual(changed.sourceFingerprint, result.sourceFingerprint);
});

test('source inspection: CLI inspects supplied files without attributing the Node architecture to Cursor', () => {
  const app = sourceFixture();
  const result = spawnSync(process.execPath, [join(directory, 'inspect-client.mjs'), app], { encoding: 'utf8', timeout: 10_000, windowsHide: true });
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
  const report = JSON.parse(result.stdout);
  assert.equal(report.cursorVersion, 'synthetic');
  assert.equal(report.architecture, null);
  assert.equal(report.conclusions.supported, false);
  assert.equal(readFileSync(join(app, 'package.json'), 'utf8'), '{"version":"synthetic"}');
  assert.equal(readdirSync(app).length, 1, 'Inspection must not modify the client directory');
});

test('source inspection regression: saved native investigation cannot be confused with runtime evidence', () => {
  const bytes = readFileSync(join(directory, '../../protocol/cursor/3.21.16/native-interface-analysis.json'));
  const report = JSON.parse(bytes);
  assert.equal(report.cursorVersion, '3.21.16');
  assert.equal(report.evidenceKind, 'sourceAnalysis');
  assert.equal(report.cursorClientExecuted, false);
  assert.equal(report.conclusions.extensionProposalAccess, 'builtinOnlyGuardObserved');
  assert.equal(report.conclusions.runtimeAccess, 'builtinExtensionIdentityGuardObserved');
  assert.equal(report.conclusions.nativeApprovalRouting, 'internalCallbackChainObserved');
  assert.equal(report.conclusions.nativeShared, 'unverified');
  assert.equal(report.conclusions.supported, false);
  const fingerprint = createHash('sha256').update(JSON.stringify(report.observations.map(({ relativePath, sha256 }) => ({ relativePath, sha256 })))).digest('hex');
  assert.equal(fingerprint, report.sourceFingerprint);
  const record = JSON.parse(readFileSync(join(directory, '../../protocol/cursor/3.21.16/Agent/compatibility.json'), 'utf8'));
  assert.equal(record.nativeAdapter.sourceInvestigation.sha256, createHash('sha256').update(bytes).digest('hex'));
  assert.equal(record.nativeAdapter.sourceInvestigation.sourceFingerprint, fingerprint);
  assert.equal(record.nativeAdapter.sourceInvestigation.nativeToolCoverageVerified, false);
  assert.equal(record.checks['P0-09'].status, 'notRun');
  assert.deepEqual(record.checks['P0-09'].evidence, []);
  assert.equal(report.observations.find(o => o.kind === 'agentHost').sha256, record.identity.clientFingerprint);
});

test('synthetic: JavaScript and Rust share business digests while new review IDs remain distinct', () => {
  const fixtures = JSON.parse(readFileSync(join(directory, '../../protocol/cursor/codecraft-v1/fixtures/business-digests.json'), 'utf8'));
  assert.equal(fixtures.evidenceKind, 'synthetic');
  assert.equal(fixtures.cursorClientExecuted, false);
  const canonical = value => {
    if (value === null || typeof value !== 'object') return JSON.stringify(value);
    if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`;
    return `{${Object.keys(value).sort().map(key => `${JSON.stringify(key)}:${canonical(value[key])}`).join(',')}}`;
  };
  const hash = value => createHash('sha256').update(canonical(value)).digest('hex');
  for (const fixture of fixtures.cases) {
    const { schemaVersion, bridgeTicket, ...businessPayload } = fixture.arguments;
    assert.equal(hash({ schemaVersion, toolName: fixture.toolName, businessPayload }), fixture.expected.argumentsDigest);
    if (fixture.toolName === 'codecraft_review_plan') {
      assert.equal(hash(Object.fromEntries(['schemaVersion', 'title', 'overview', 'markdown', 'todos', 'phases'].map(key => [key, fixture.arguments[key]]))), fixture.expected.planContentDigest);
    }
  }
  const cases = Object.fromEntries(fixtures.cases.map(fixture => [fixture.name, fixture.expected]));
  assert.equal(cases['plan-original'].planContentDigest, cases['plan-new-review'].planContentDigest);
  assert.notEqual(cases['plan-original'].argumentsDigest, cases['plan-new-review'].argumentsDigest);
  assert.notEqual(cases['plan-original'].planContentDigest, cases['plan-newline'].planContentDigest);
  assert.deepEqual(cases['plan-original'], cases['plan-ticket']);
});

test('reviewed extension runtime: visible names and a rejected builtin API never grant native approval', () => {
  const report = JSON.parse(readFileSync(join(directory, '../../protocol/cursor/3.21.16/extension-access-runtime.json'), 'utf8'));
  assert.equal(report.evidenceKind, 'cursorExtensionRuntime');
  assert.equal(report.cursorClientExecuted, true);
  assert.equal(report.cursorVersion, '3.21.16');
  assert.equal(report.architectureSource, 'cursorExecutablePeHeader');
  assert.equal(report.isolatedProfile, true);
  assert.equal(report.accountSessionUsed, false);
  assert.equal(report.clientPatched, false);
  assert.equal(report.capabilityEnabled, false);
  assert.equal(report.result.extensionId, 'codecraft.cursor-api-access-probe');
  assert.equal(report.result.extensionMode, 3);
  assert.equal(report.result.namespace.present, true);
  assert.ok(report.result.namespace.keys.includes('acquireAgentHostRuntime'));
  assert.deepEqual(report.result.namespace.runtime, { acquired: false, error: 'BUILTIN_ONLY' });
  assert.equal(report.result.nativeRequestRead, false);
  assert.equal(report.result.nativeDecisionSubmitted, false);
  assert.equal(report.result.supported, false);
  const hash = path => createHash('sha256').update(readFileSync(path)).digest('hex');
  assert.equal(report.runnerSha256, hash(join(directory, 'verify-extension-access.mjs')));
  for (const source of report.probeFiles) assert.equal(source.sha256, hash(join(directory, 'extension-access', source.file)));
  const compatibility = JSON.parse(readFileSync(join(directory, '../../protocol/cursor/3.21.16/Agent/compatibility.json'), 'utf8'));
  assert.notEqual(compatibility.checks['P0-09'].status, 'passed');
});
