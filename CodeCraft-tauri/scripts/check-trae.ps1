param([switch]$Full)
$ErrorActionPreference = 'Stop'
(Get-Process -Id $PID).ProcessorAffinity = 3
$env:CARGO_BUILD_JOBS = '2'
Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    & cargo test --locked --manifest-path src-tauri/trae-core/Cargo.toml -- --test-threads=2
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & cargo clippy --locked --manifest-path src-tauri/trae-core/Cargo.toml --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & cargo fmt --manifest-path src-tauri/trae-core/Cargo.toml --check
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & node scripts/generate-trae-types.mjs --check
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & cargo build --locked --manifest-path src-tauri/trae-core/Cargo.toml --example bridge_test_host
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & node scripts/verify-trae-core.mjs
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & node scripts/verify-trae-transport.mjs
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & node scripts/verify-trae-transport.mjs --bundled
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    if ($Full) {
        # Invoke in a child PowerShell because the probe script uses exit.
        & powershell -NoProfile -ExecutionPolicy Bypass -File scripts/trae-probe/check.ps1
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        & cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=2
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        & npm test -- --maxWorkers=2
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        & npm run build
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        & cargo build --locked --manifest-path src-tauri/Cargo.toml
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        & node scripts/verify-trae-chat.mjs
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
} finally { Pop-Location }
