param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
(Get-Process -Id $PID).ProcessorAffinity = 3
$env:CARGO_BUILD_JOBS = '2'
$probeManifest = Join-Path $PSScriptRoot 'Cargo.toml'
if (-not $SkipBuild) {
    & cargo test --locked --manifest-path $probeManifest -- --test-threads=2
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & cargo build --locked --manifest-path $probeManifest
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
& node (Join-Path $PSScriptRoot 'verify.mjs')
exit $LASTEXITCODE

