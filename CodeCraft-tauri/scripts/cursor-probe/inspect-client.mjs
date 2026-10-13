// Read-only installed-client reconnaissance. Source analysis is never runtime evidence.
import { readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';

const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const sources = [
  ['agentHost', 'extensions/cursor-agent-host/dist/main.js', [
    ['mcpCandidateName', 'const h=`MCP:${t.toolName}`'],
    ['mcpServerIdentity', 'mcp_server_name:t.providerIdentifier'],
    ['requestApproval', 'pendingDecisionProvider.requestApproval'],
    ['updatedInput', 'updated_input'],
    ['failureEvent', 'firePostToolUseFailureAsync'],
    ['interactionResponse', 'interactionRegistry.respond('],
    ['interactionWatch', 'interactionRegistry.watch('],
    ['interactionStatuses', '"AgentHostInteractionResponseStatus"'],
    ['interactionConsumption', 'this.pending.delete(e),this.resolved.set(e,{case:"responded"'],
  ]],
  ['agentExec', 'extensions/cursor-agent-exec/dist/main.js', [
    ['approvalForwarding', 'async requestApproval(e){if(e.toolCallId)'],
  ]],
  ['extensionHost', 'out/vs/workbench/api/node/extensionHostProcess.js', [
    ['builtinProposalGuard', /return\([^)]*"cursorAgentHost"[^)]*\)&&![\w$]+\.isBuiltin/],
    ['builtinProposalDiagnostic', 'proposals are only available for built-in extensions'],
    ['builtinIdentityGuard', /if\([\w$]+\([\w$]+,"cursorAgentHost"\),![\w$]+\.isBuiltin\|\|![\w$]+\.equals\(/],
    ['runtimeAccessGuard', /acquireAgentHostRuntime\(\)\{return [\w$]+\([\w$]+,[\w$]+,"acquireAgentHostRuntime"\)/],
    ['approvalRpc', 'this._proxy.$requestAgentExecApproval('],
    ['interactionRpc', '$respondToAgentHostInteraction('],
  ]],
  ['workbench', 'out/vs/workbench/workbench.desktop.main.js', [
    ['approvalHandler', '$requestAgentExecApproval('],
    ['interactionMirror', '[AgentHostInteractionMirror] agent host rejected interaction response'],
  ]],
  ['extensionApi', 'out/vscode-dts/vscode.d.ts', [
    ['cursorNamespace', 'namespace cursor'],
    ['approvalIdDeclaration', 'nativeApprovalId'],
    ['approvalSubmitDeclaration', 'submitApproval'],
  ]],
];

export function inspectClient(app, inspectedAt = new Date().toISOString()) {
  const metadataBytes = readFileSync(join(app, 'package.json'));
  const metadata = JSON.parse(metadataBytes.toString('utf8'));
  const observations = sources.map(([kind, relativePath, patterns]) => {
    let bytes;
    try {
      bytes = readFileSync(join(app, relativePath));
    } catch (error) {
      // A moved/removed bundle is an unknown layout, not proof that its APIs
      // or identity fields do not exist. Never inherit the old conclusion.
      if (error.code !== 'ENOENT' && error.code !== 'ENOTDIR') throw error;
      return { kind, relativePath, status: 'missing', sha256: null, findings: [] };
    }
    const text = bytes.toString('utf8');
    return {
      kind, relativePath, status: 'read', sha256: hash(bytes),
      findings: patterns.map(([id, pattern]) => {
        const match = typeof pattern === 'string' ? null : pattern.exec(text);
        const offset = typeof pattern === 'string' ? text.indexOf(pattern) : (match?.index ?? -1);
        const needle = typeof pattern === 'string' ? pattern : (match?.[0] ?? null);
        return {
          id, pattern: String(pattern), needle, found: offset >= 0, offset,
          excerpt: offset >= 0 ? text.slice(Math.max(0, offset - 120), offset + needle.length + 550) : null,
        };
      }),
    };
  });
  const found = (kind, id) => observations.find(o => o.kind === kind)?.findings.some(f => f.id === id && f.found) ?? false;
  const nativeApprovalRouting = found('agentHost', 'requestApproval')
    && found('agentExec', 'approvalForwarding')
    && found('extensionHost', 'approvalRpc')
    && found('workbench', 'approvalHandler');
  const guardedProposal = found('extensionHost', 'builtinProposalGuard')
    && found('extensionHost', 'builtinProposalDiagnostic');
  const guardedRuntime = guardedProposal && found('extensionHost', 'builtinIdentityGuard')
    && found('extensionHost', 'runtimeAccessGuard');
  return {
    schemaVersion: 2, evidenceKind: 'sourceAnalysis', cursorClientExecuted: false,
    cursorVersion: metadata.version, platform: 'windows', architecture: metadata.target ?? null,
    inspectedAt, metadataSha256: hash(metadataBytes), offsetEncoding: 'utf16CodeUnits',
    sourceFingerprint: hash(JSON.stringify(observations.map(({ relativePath, sha256 }) => ({ relativePath, sha256 })))),
    observations,
    conclusions: {
      mcpCandidateIdentity: found('agentHost', 'mcpCandidateName') ? 'unqualifiedWrapperObserved' : 'unknown',
      nativeApprovalRouting: nativeApprovalRouting ? 'internalCallbackChainObserved' : 'unknown',
      extensionProposalAccess: guardedProposal ? 'builtinOnlyGuardObserved' : 'unknown',
      runtimeAccess: guardedRuntime ? 'builtinExtensionIdentityGuardObserved' : 'unknown',
      interactionRegistry: found('agentHost', 'interactionResponse') && found('agentHost', 'interactionWatch')
        ? 'internalRespondAndWatchObserved' : 'unknown',
      nativeShared: 'unverified', supported: false,
      limitations: [
        'Source markers are investigation leads, never Cursor runtime or P0 pass evidence.',
        'Missing markers do not prove an API is absent or publicly accessible; inspect changed bundles again.',
        'Interaction registry status values do not prove tool approval coverage or a reachable external adapter.',
        'No native request submission, acceptance, cancellation, concurrent arbitration or reconnection was executed.',
      ],
    },
  };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.length !== 3) throw new Error('Usage: node inspect-client.mjs <Cursor resources/app>');
  console.log(JSON.stringify(inspectClient(process.argv[2]), null, 2));
}
