const fs = require('node:fs');
const vscode = require('vscode');

exports.run = async () => {
  const path = process.env.CODECRAFT_CURSOR_ACCESS_REPORT;
  if (!path) throw new Error('Missing isolated report path');
  const extension = vscode.extensions.getExtension('codecraft.cursor-api-access-probe');
  if (!extension) throw new Error('Probe extension was not loaded');
  const result = await extension.activate();
  fs.writeFileSync(path, JSON.stringify(result, null, 2), { flag: 'wx' });
};
