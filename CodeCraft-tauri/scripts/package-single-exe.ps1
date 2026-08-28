$ErrorActionPreference = "Stop"

# Keep Rust and its child processes on two logical CPUs for predictable builds.
$process = [System.Diagnostics.Process]::GetCurrentProcess()
$process.ProcessorAffinity = [IntPtr]3
$env:CARGO_BUILD_JOBS = "2"

$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $projectRoot
try {
    npm run tauri -- build --no-bundle
    if ($LASTEXITCODE -ne 0) {
        throw "Tauri build failed with exit code $LASTEXITCODE."
    }

    $binary = Join-Path $projectRoot "src-tauri\target\release\codecraft-tauri.exe"
    if (-not (Test-Path -LiteralPath $binary)) {
        throw "Build completed but the executable was not found at $binary."
    }

    $artifactDirectory = Join-Path $projectRoot "artifacts"
    New-Item -ItemType Directory -Force -Path $artifactDirectory | Out-Null
    $artifact = Join-Path $artifactDirectory "CodeCraft.exe"
    Copy-Item -LiteralPath $binary -Destination $artifact -Force

    $sizeMb = [Math]::Round((Get-Item -LiteralPath $artifact).Length / 1MB, 1)
    Write-Host "Single-file executable created: $artifact ($sizeMb MB)"
}
finally {
    Pop-Location
}
