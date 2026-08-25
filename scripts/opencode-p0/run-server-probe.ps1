param(
    [string]$OpenCodePath,
    [ValidatePattern("^[A-Za-z0-9._-]+$")]
    [string]$ProbeName = "server-runtime",
    [ValidateRange(1024, 65535)]
    [int]$Port = 48765
)

$ErrorActionPreference = "Stop"

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Definition
$probeRoot = Join-Path $env:TEMP "codecraft-opencode-p0-$ProbeName"
$configDir = Join-Path $probeRoot "config"
$dataDir = Join-Path $probeRoot "data"
$cacheDir = Join-Path $probeRoot "cache"
$stateDir = Join-Path $probeRoot "state"
$pluginDir = Join-Path $configDir "plugins"
$logPath = Join-Path $probeRoot "probe.jsonl"
$stdoutPath = Join-Path $probeRoot "opencode.stdout.log"
$stderrPath = Join-Path $probeRoot "opencode.stderr.log"

New-Item -ItemType Directory -Force -Path $pluginDir,$dataDir,$cacheDir,$stateDir | Out-Null
Copy-Item -LiteralPath (Join-Path $scriptRoot "probe-plugin.js") -Destination (Join-Path $pluginDir "codecraft-p0-probe.js") -Force
Remove-Item -LiteralPath $logPath,$stdoutPath,$stderrPath -Force -ErrorAction SilentlyContinue

$env:OPENCODE_CONFIG_DIR = $configDir
$env:XDG_DATA_HOME = $dataDir
$env:XDG_CACHE_HOME = $cacheDir
$env:XDG_STATE_HOME = $stateDir
$env:OPENCODE_DISABLE_AUTOUPDATE = "true"
$env:OPENCODE_CONFIG_CONTENT = '{"agent":{"build":{"permission":{"*":"allow","external_directory":"deny"}}}}'
$env:CODECRAFT_P0_LOG = $logPath
$env:CODECRAFT_P0_BEFORE_MODE = "observe"

$opencode = $OpenCodePath
if ([string]::IsNullOrWhiteSpace($opencode)) {
    $opencode = "E:\nodejs\node_global\node_modules\opencode-ai\bin\opencode.exe"
    if (-not (Test-Path -LiteralPath $opencode)) {
        $opencode = (Get-Command opencode -ErrorAction Stop).Source
    }
}
$opencode = (Resolve-Path -LiteralPath $opencode -ErrorAction Stop).Path

$process = Start-Process -FilePath $opencode -ArgumentList @("serve", "--hostname", "127.0.0.1", "--port", $Port) -PassThru -NoNewWindow -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
try {
    $process.ProcessorAffinity = [IntPtr]3
    $ready = $false
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        Start-Sleep -Milliseconds 250
        try {
            $health = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/global/health" -TimeoutSec 2
            if ($health) { $ready = $true; break }
        } catch {}
    }
    if (-not $ready) { throw "OpenCode serve did not become ready." }

    $directory = (Get-Location).Path
    $headers = @{ "x-opencode-directory" = [Uri]::EscapeDataString($directory) }
    try {
        Invoke-RestMethod -Uri "http://127.0.0.1:$Port/path" -Headers $headers -TimeoutSec 30 | Out-Null
    } catch {
        Write-Warning "Instance path probe failed: $($_.Exception.Message)"
    }
    Start-Sleep -Milliseconds 500
} finally {
    if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
}

Write-Host "Probe root: $probeRoot"
Write-Host "OpenCode executable: $opencode"
if (Test-Path -LiteralPath $logPath) {
    Get-Content -LiteralPath $logPath
} else {
    Write-Warning "Probe plugin did not create a log file."
}
if (Test-Path -LiteralPath $stderrPath) { Get-Content -LiteralPath $stderrPath }
if (Test-Path -LiteralPath $stdoutPath) { Get-Content -LiteralPath $stdoutPath }
