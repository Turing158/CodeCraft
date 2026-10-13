// Actual Cursor extension-host access probe, isolated from the user's profile.
// An access failure is evidence about this route, never a passing P0 capability.
// Run after dot-sourcing limit-cpu.ps1; child processes inherit its two-core mask.
import { spawn } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync, existsSync, openSync, readSync, closeSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const directory = dirname(fileURLToPath(import.meta.url));
const executable = resolve(process.argv[2] ?? 'D:/software/cursor/Cursor.exe');
const extension = join(directory, 'extension-access');
const hash = path => createHash('sha256').update(readFileSync(path)).digest('hex');
const sourcePaths = [executable, fileURLToPath(import.meta.url),
  ...['package.json', 'extension.cjs', 'tests.cjs'].map(file => join(extension, file)),
  join(dirname(executable), 'resources/app/package.json')];
const sourceHashes = new Map(sourcePaths.map(path => [path, hash(path)]));
const run = join(directory, 'runs', `extension-access-${randomUUID()}`);
for (const name of ['profile', 'extensions', 'workspace', 'home', 'local', 'roaming', 'temp']) mkdirSync(join(run, name), { recursive: true });
const output = join(run, 'access.json');
const args = [
  `--user-data-dir=${join(run, 'profile')}`, `--extensions-dir=${join(run, 'extensions')}`,
  `--extensionDevelopmentPath=${extension}`, `--extensionTestsPath=${join(extension, 'tests.cjs')}`,
  '--enable-proposed-api=codecraft.cursor-api-access-probe',
  '--skip-welcome', '--skip-release-notes', '--skip-onboarding', '--disable-updates',
  join(run, 'workspace'),
];
const env = { ...process.env, USERPROFILE: join(run, 'home'), LOCALAPPDATA: join(run, 'local'),
  APPDATA: join(run, 'roaming'), TEMP: join(run, 'temp'), TMP: join(run, 'temp'), CODECRAFT_CURSOR_ACCESS_REPORT: output };
delete env.ELECTRON_RUN_AS_NODE;
const child = spawn(executable, args, { env, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
let bytes = 0;
const chunks = [];
const capture = data => { if (bytes < 1024 * 1024) { chunks.push(data); bytes += data.length; } };
child.stdout.on('data', capture);
child.stderr.on('data', capture);
const timer = setTimeout(() => {
  if (child.exitCode === null) {
    // Terminate only the still-running, isolated test process and its children.
    spawn('taskkill', ['/PID', String(child.pid), '/T', '/F'], { windowsHide: true, stdio: 'ignore' });
  }
}, 120000);
let code;
try {
  code = await new Promise((accept, reject) => { child.once('error', reject); child.once('exit', accept); });
} finally { clearTimeout(timer); }
writeFileSync(join(run, 'process.log'), Buffer.concat(chunks));
if (code !== 0 || !existsSync(output)) throw new Error(`Extension probe did not complete (exit ${code}); see ${run}`);
for (const [path, initialHash] of sourceHashes) {
  if (hash(path) !== initialHash) throw new Error('Cursor or probe source changed during the experiment; rerun instead of publishing this report');
}
const executableArchitecture = path => {
  const file = openSync(path, 'r');
  try {
    const dos = Buffer.alloc(64);
    if (readSync(file, dos, 0, dos.length, 0) !== 64 || dos.toString('ascii', 0, 2) !== 'MZ') return null;
    const header = Buffer.alloc(6);
    if (readSync(file, header, 0, header.length, dos.readUInt32LE(60)) !== 6 || header.readUInt32LE(0) !== 0x4550) return null;
    return ({ 0x8664: 'x64', 0xaa64: 'arm64', 0x14c: 'x86' })[header.readUInt16LE(4)] ?? null;
  } finally { closeSync(file); }
};
const report = {
  schemaVersion: 1, evidenceKind: 'cursorExtensionRuntime', cursorClientExecuted: true,
  experiment: 'ordinary-development-extension-access', capturedAt: new Date().toISOString(),
  cursorVersion: JSON.parse(readFileSync(join(dirname(executable), 'resources/app/package.json'), 'utf8')).version,
  platform: 'windows', architecture: executableArchitecture(executable), architectureSource: 'cursorExecutablePeHeader',
  executableSha256: sourceHashes.get(executable),
  runnerSha256: sourceHashes.get(fileURLToPath(import.meta.url)),
  probeFiles: ['package.json', 'extension.cjs', 'tests.cjs'].map(file => ({ file, sha256: sourceHashes.get(join(extension, file)) })),
  isolatedProfile: true, accountSessionUsed: false, proposedApiFlagUsed: true, clientPatched: false, capabilityEnabled: false,
  result: JSON.parse(readFileSync(output, 'utf8')),
};
writeFileSync(join(run, 'report.json'), JSON.stringify(report, null, 2));
console.log(JSON.stringify({ run, reportFile: join(run, 'report.json'), cursorVersion: report.cursorVersion,
  architecture: report.architecture, runtimeAccess: report.result.namespace.runtime,
  builtinExports: report.result.builtinExports, relatedCommandIds: report.result.relatedCommandIds,
  capabilityEnabled: false }, null, 2));
