import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { fixHookReader, patchCursorHookReader } from './fix-hook-encoding.mjs';

const wrapper = "$OutputEncoding = [System.Text.Encoding]::UTF8; Get-Content -LiteralPath '${a}' -Raw | & { $input | ${r} }";
const root = fileURLToPath(new URL('./runs/', import.meta.url));
mkdirSync(root, { recursive: true });

test('changes exactly the known Hook reader and rejects unknown/ambiguous versions', () => {
  const fixed = fixHookReader(`before\n${wrapper}\nafter`);
  assert.equal(fixed, `before\n${wrapper.replace('Get-Content ', 'Get-Content -Encoding UTF8 ')}\nafter`);
  assert.equal(fixHookReader(fixed), fixed);
  assert.throws(() => fixHookReader('unknown layout'));
  assert.throws(() => fixHookReader(wrapper + wrapper));
  assert.throws(() => fixHookReader(wrapper + fixed));
});

test('saves the exact original as .bak and is safe to run twice', () => {
  const directory = mkdtempSync(join(root, 'encoding-backup-'));
  const target = join(directory, 'out/vs/workbench/api/node/extensionHostProcess.js');
  mkdirSync(join(directory, 'out/vs/workbench/api/node'), { recursive: true });
  writeFileSync(target, wrapper);
  const result = patchCursorHookReader(directory);
  assert.equal(result.status, 'fixed');
  assert.ok(result.backup.endsWith('.bak'));
  assert.equal(readFileSync(result.backup, 'utf8'), wrapper);
  assert.equal(readFileSync(target, 'utf8'), fixHookReader(wrapper));
  assert.equal(patchCursorHookReader(directory).status, 'alreadyFixed');
});

test('Windows PowerShell Hook pipeline preserves Chinese, emoji and nested tool JSON', { skip: process.platform !== 'win32' }, () => {
  const directory = mkdtempSync(join(root, 'encoding-pipeline-'));
  const payloadPath = join(directory, 'payload.json');
  const payload = JSON.stringify({ tool_name: 'codecraft_start_workflow', tool_input: JSON.stringify({
    projectId: 'synthetic-project', title: '问答界面验证', content: '计划界面验证\n中文路径：D:/项目/说明.md 😀',
  }) });
  writeFileSync(payloadPath, payload, 'utf8');
  const quote = text => text.replaceAll("'", "''");
  const command = fixHookReader(wrapper)
    .replace('${a}', quote(payloadPath))
    .replace('${r}', `& '${quote(process.execPath)}' -e 'process.stdin.pipe(process.stdout)'`);
  const result = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', command], {
    encoding: 'utf8', timeout: 10000, windowsHide: true,
  });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout.replace(/^\uFEFF/, '').trimEnd(), payload);
});
