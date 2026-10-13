// Work around Cursor's Windows PowerShell Hook reader until upstream specifies
// UTF-8. Only the known wrapper is changed, with a same-directory .bak for undo.
import { constants, copyFileSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';

const original = '$OutputEncoding = [System.Text.Encoding]::UTF8; Get-Content -LiteralPath';
const corrected = '$OutputEncoding = [System.Text.Encoding]::UTF8; Get-Content -Encoding UTF8 -LiteralPath';

export function fixHookReader(source) {
  const before = source.split(original).length - 1;
  const after = source.split(corrected).length - 1;
  if (before === 0 && after === 1) return source;
  if (before !== 1 || after !== 0) throw new Error('Unknown Cursor Hook reader; no files changed.');
  return source.replace(original, corrected);
}

export function patchCursorHookReader(appDirectory) {
  const target = join(resolve(appDirectory), 'out/vs/workbench/api/node/extensionHostProcess.js');
  const bytes = readFileSync(target);
  const source = bytes.toString('utf8');
  if (!Buffer.from(source, 'utf8').equals(bytes)) throw new Error('Invalid source encoding; no files changed.');
  const updated = fixHookReader(source);
  if (updated === source) return { status: 'alreadyFixed', target };
  const backup = `${target}.codecraft-utf8-${Date.now()}.bak`;
  copyFileSync(target, backup, constants.COPYFILE_EXCL);
  if (!readFileSync(backup).equals(bytes) || !readFileSync(target).equals(bytes)) {
    throw new Error('Source changed while backing up; target was not overwritten.');
  }
  writeFileSync(target, updated, 'utf8');
  if (readFileSync(target, 'utf8') !== updated) throw new Error(`Write verification failed; restore ${backup}`);
  return { status: 'fixed', target, backup };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  if (!process.argv[2]) throw new Error('Usage: node fix-hook-encoding.mjs <Cursor resources/app directory>');
  console.log(JSON.stringify(patchCursorHookReader(process.argv[2]), null, 2));
}
