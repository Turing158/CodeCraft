param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'limit-cpu.ps1')
if (-not $SkipBuild) {
    & cargo build --locked --manifest-path (Join-Path $PSScriptRoot 'Cargo.toml')
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
$cursorProbeRuns = Join-Path $PSScriptRoot 'runs'
New-Item -ItemType Directory -Path $cursorProbeRuns -Force | Out-Null
$cursorProbeRun = Join-Path $cursorProbeRuns ([guid]::NewGuid().ToString())
& (Join-Path $PSScriptRoot 'target\debug\codecraft-cursor-probe.exe') init --root $cursorProbeRun
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Write-Output "Open only this isolated Cursor project: $(Join-Path $cursorProbeRun 'project')"
Write-Output 'User configuration is unchanged. This prepares observation; it does not pass P0 or enable CodeCraft.'
