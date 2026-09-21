param()
$ErrorActionPreference = 'Stop'
(Get-Process -Id $PID).ProcessorAffinity = 3
$env:CARGO_BUILD_JOBS = '2'
& cargo build --locked --manifest-path (Join-Path $PSScriptRoot 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$probeExecutable = Join-Path $PSScriptRoot 'target\debug\codecraft-trae-probe.exe'
$probeRuns = Join-Path $PSScriptRoot 'runs'
New-Item -ItemType Directory -Path $probeRuns -Force | Out-Null
$probeRun = Join-Path $probeRuns ([guid]::NewGuid().ToString())
& $probeExecutable init --root $probeRun
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$probeProcess = Start-Process -FilePath $probeExecutable -ArgumentList @('coordinator', '--root', ('"' + $probeRun + '"')) -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $probeRun 'coordinator.stdout.log') -RedirectStandardError (Join-Path $probeRun 'coordinator.stderr.log')
$probeProcess.ProcessorAffinity = 3
$probeInfo = @{ pid = $probeProcess.Id; createdAt = $probeProcess.StartTime.ToUniversalTime().ToString('o'); executable = $probeExecutable; root = $probeRun }
$probeInfo | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $probeRun 'coordinator-process.json') -Encoding utf8
Write-Output "P0 probe prepared. Open the project directory in Trae; follow protocol/trae/3.3.102/README.md before enabling its project Hook."

