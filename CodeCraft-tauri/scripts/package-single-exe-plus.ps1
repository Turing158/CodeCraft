param(
    # Skip the UPX post-compression pass (also skips the automatic UPX download).
    [switch]$SkipUpx,
    # Let UPX try every method/filter combination. Much slower, occasionally a bit smaller.
    [switch]$UltraBrute
)

$ErrorActionPreference = "Stop"

# Keep Rust and its child processes on two logical CPUs for predictable builds.
$process = [System.Diagnostics.Process]::GetCurrentProcess()
$process.ProcessorAffinity = [IntPtr]3
$env:CARGO_BUILD_JOBS = "2"

# Size-first release profile injected through the environment so Cargo.toml stays
# untouched and regular dev builds keep their own cache fingerprint:
#   opt-level=z    - optimize for size instead of speed
#   lto=fat        - whole-program link-time optimization across all crates
#   codegen-units=1- single codegen unit so LTO can see everything
#   panic=abort    - drops unwinding tables (the app has no catch_unwind users)
#   strip=symbols  - removes the COFF symbol table from the final binary
$env:CARGO_PROFILE_RELEASE_OPT_LEVEL = "z"
$env:CARGO_PROFILE_RELEASE_LTO = "fat"
$env:CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "1"
$env:CARGO_PROFILE_RELEASE_PANIC = "abort"
$env:CARGO_PROFILE_RELEASE_STRIP = "symbols"
$env:CARGO_PROFILE_RELEASE_DEBUG = "0"
$env:CARGO_INCREMENTAL = "0"

$projectRoot = Split-Path -Parent $PSScriptRoot

function Get-UpxExecutable {
    # Returns a usable upx.exe from PATH, the local tool cache, or a fresh
    # GitHub download; returns $null when UPX cannot be provided.
    $command = Get-Command "upx.exe" -ErrorAction SilentlyContinue
    if ($null -ne $command) { return $command.Source }

    $toolDirectory = Join-Path $env:LOCALAPPDATA "CodeCraft\tools\upx"
    $cached = Join-Path $toolDirectory "upx.exe"
    if (Test-Path -LiteralPath $cached) { return $cached }

    Write-Host "upx.exe not found; downloading the latest UPX win64 release from GitHub..."
    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        $release = Invoke-RestMethod -Uri "https://api.github.com/repos/upx/upx/releases/latest" -UseBasicParsing
        $asset = @($release.assets) |
            Where-Object { $_.name -match "^upx-.*-win64\.zip$" } |
            Select-Object -First 1
        if ($null -eq $asset) {
            throw "no upx-*-win64.zip asset found in the latest release"
        }

        New-Item -ItemType Directory -Force -Path $toolDirectory | Out-Null
        $zipPath = Join-Path $toolDirectory $asset.name
        Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zipPath -UseBasicParsing
        Expand-Archive -LiteralPath $zipPath -DestinationPath $toolDirectory -Force

        $extracted = Get-ChildItem -LiteralPath $toolDirectory -Recurse -Filter "upx.exe" |
            Select-Object -First 1
        if ($null -eq $extracted) {
            throw "upx.exe missing from the downloaded archive"
        }
        Move-Item -LiteralPath $extracted.FullName -Destination $cached -Force
        Remove-Item -LiteralPath $zipPath -Force -ErrorAction SilentlyContinue
        Get-ChildItem -LiteralPath $toolDirectory -Directory | Remove-Item -Recurse -Force
        return $cached
    }
    catch {
        Write-Warning "Could not provide UPX ($($_.Exception.Message)); continuing without compression."
        return $null
    }
}

function Compress-WithUpx {
    param(
        [Parameter(Mandatory = $true)][string]$UpxPath,
        [Parameter(Mandatory = $true)][string]$ExecutablePath,
        [switch]$UltraBrute
    )

    # Compress a staged copy so a failed pass never damages the artifact.
    $staged = "$ExecutablePath.upx-staging"
    Copy-Item -LiteralPath $ExecutablePath -Destination $staged -Force

    $upxArguments = @("--best", "--lzma")
    if ($UltraBrute) { $upxArguments += "--ultra-brute" }
    $upxArguments += $staged

    & $UpxPath @upxArguments
    $upxExitCode = $LASTEXITCODE

    if ($upxExitCode -eq 0 -and (Test-Path -LiteralPath $staged)) {
        Move-Item -LiteralPath $staged -Destination $ExecutablePath -Force
        return $true
    }

    if (Test-Path -LiteralPath $staged) { Remove-Item -LiteralPath $staged -Force }
    Write-Warning "UPX exited with code $upxExitCode; keeping the uncompressed executable."
    return $false
}

Push-Location $projectRoot
try {
    # The artifact and the release binary are overwritten in place during the
    # build, so a still-running instance would fail the copy or the linker.
    $running = @(Get-Process -Name "CodeCraft", "codecraft-tauri" -ErrorAction SilentlyContinue)
    if ($running.Count -gt 0) {
        $details = ($running | ForEach-Object { "$($_.ProcessName) (PID $($_.Id))" }) -join ", "
        throw "An instance is still running: $details. Close it before packaging."
    }

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
    $uncompressedBytes = (Get-Item -LiteralPath $artifact).Length

    $compressed = $false
    if (-not $SkipUpx) {
        $upxTool = Get-UpxExecutable
        if ($null -ne $upxTool) {
            $compressed = Compress-WithUpx -UpxPath $upxTool -ExecutablePath $artifact -UltraBrute:$UltraBrute
        }
    }

    $finalBytes = (Get-Item -LiteralPath $artifact).Length
    $finalMb = [Math]::Round($finalBytes / 1MB, 2)
    Write-Host "Single-file executable created: $artifact ($finalMb MB)"
    if ($compressed) {
        $uncompressedMb = [Math]::Round($uncompressedBytes / 1MB, 2)
        $savedPercent = [Math]::Round(100 - ($finalBytes / $uncompressedBytes * 100), 1)
        Write-Host "Uncompressed build: $uncompressedMb MB -> packed: $finalMb MB ($savedPercent% smaller)."
        Write-Host "Note: UPX-packed executables are occasionally flagged by antivirus heuristics; use -SkipUpx for an unpacked build."
    }
}
finally {
    Pop-Location
}
