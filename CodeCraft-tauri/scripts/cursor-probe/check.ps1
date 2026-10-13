param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'limit-cpu.ps1')
if (-not $SkipBuild) {
    $cursorCore = Join-Path $PSScriptRoot '../../src-tauri/cursor-core/Cargo.toml'
    & cargo fmt --manifest-path $cursorCore --check
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & cargo test --locked --manifest-path $cursorCore -- --test-threads=2
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & cargo clippy --locked --manifest-path $cursorCore --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & cargo fmt --manifest-path (Join-Path $PSScriptRoot 'Cargo.toml') --check
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & cargo test --locked --manifest-path (Join-Path $PSScriptRoot 'Cargo.toml') -- --test-threads=2
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & cargo build --locked --manifest-path (Join-Path $PSScriptRoot 'Cargo.toml')
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & cargo clippy --locked --manifest-path (Join-Path $PSScriptRoot 'Cargo.toml') --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
& node --test --test-concurrency=2 (Join-Path $PSScriptRoot 'verify.mjs')
exit $LASTEXITCODE
