// A normal development extension: no patches, identity impersonation or UI actions.
const vscode = require('vscode');

const accessError = error => {
  const message = String(error?.message ?? error);
  if (/only available for built-in extensions/.test(message)) return 'BUILTIN_ONLY';
  if (/proposed API|API proposal|enabledApiProposals/.test(message)) return 'PROPOSAL_DENIED';
  return 'UNAVAILABLE';
};

exports.activate = async context => {
  const result = {
    extensionId: context.extension.id,
    extensionMode: context.extensionMode,
    vscodeVersion: vscode.version,
    namespace: {},
    builtinExports: [],
    relatedCommandIds: [],
    nativeRequestRead: false,
    nativeDecisionSubmitted: false,
    supported: false,
  };
  try {
    const cursor = vscode.cursor;
    result.namespace = { present: cursor != null, keys: cursor ? Object.keys(cursor).sort() : [] };
    if (cursor && typeof cursor.acquireAgentHostRuntime === 'function') {
      try {
        const runtime = await cursor.acquireAgentHostRuntime();
        result.namespace.runtime = { acquired: runtime != null, exportedKeys: runtime ? Object.keys(runtime).sort() : [] };
        // Do not register a replacement or mutate a returned runtime.
      } catch (error) { result.namespace.runtime = { acquired: false, error: accessError(error) }; }
    }
  } catch (error) { result.namespace = { present: false, error: accessError(error) }; }
  for (const id of ['anysphere.cursor-agent-host', 'anysphere.cursor-agent-exec', 'anysphere.cursor-local-agent-runtime']) {
    const extension = vscode.extensions.getExtension(id);
    const entry = { id, present: extension != null, active: extension?.isActive ?? false, exportedKeys: [] };
    if (extension) {
      try {
        const value = await extension.activate();
        entry.active = extension.isActive;
        entry.exportedKeys = value ? Object.keys(value).sort() : [];
      } catch (error) { entry.error = accessError(error); }
    }
    result.builtinExports.push(entry);
  }
  result.relatedCommandIds = (await vscode.commands.getCommands(true))
    .filter(id => /agentHost|respondToInteraction|requestAgentExecApproval|submitApproval|nativeApproval/i.test(id))
    .sort();
  return result;
};
