param(
    [string]$OpenCodePath,
    [ValidatePattern("^[A-Za-z0-9._-]+$")]
    [string]$ProbeName = "http-contract",
    [ValidateRange(1024, 65535)]
    [int]$Port = 48767
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Net.Http

$probeRoot = Join-Path $env:TEMP "codecraft-opencode-p0-$ProbeName"
$configDir = Join-Path $probeRoot "config"
$dataDir = Join-Path $probeRoot "data"
$cacheDir = Join-Path $probeRoot "cache"
$stateDir = Join-Path $probeRoot "state"
$stdoutPath = Join-Path $probeRoot "opencode.stdout.log"
$stderrPath = Join-Path $probeRoot "opencode.stderr.log"
$resultPath = Join-Path $probeRoot "http-result.json"

New-Item -ItemType Directory -Force -Path $configDir,$dataDir,$cacheDir,$stateDir | Out-Null
Remove-Item -LiteralPath $stdoutPath,$stderrPath,$resultPath -Force -ErrorAction SilentlyContinue

$env:OPENCODE_CONFIG_DIR = $configDir
$env:XDG_DATA_HOME = $dataDir
$env:XDG_CACHE_HOME = $cacheDir
$env:XDG_STATE_HOME = $stateDir
$env:OPENCODE_DISABLE_AUTOUPDATE = "true"

if ([string]::IsNullOrWhiteSpace($OpenCodePath)) {
    $OpenCodePath = "E:\nodejs\node_global\node_modules\opencode-ai\bin\opencode.exe"
    if (-not (Test-Path -LiteralPath $OpenCodePath)) {
        $OpenCodePath = (Get-Command opencode -ErrorAction Stop).Source
    }
}
$OpenCodePath = (Resolve-Path -LiteralPath $OpenCodePath -ErrorAction Stop).Path

$process = Start-Process -FilePath $OpenCodePath -ArgumentList @("serve", "--hostname", "127.0.0.1", "--port", $Port) -PassThru -NoNewWindow -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
try {
    $process.ProcessorAffinity = [IntPtr]3
    $client = [System.Net.Http.HttpClient]::new()
    $client.DefaultRequestHeaders.Add("x-opencode-directory", [Uri]::EscapeDataString((Get-Location).Path))
    $ready = $false
    for ($attempt = 0; $attempt -lt 40; $attempt++) {
        Start-Sleep -Milliseconds 250
        try {
            $health = $client.GetAsync("http://127.0.0.1:$Port/global/health").GetAwaiter().GetResult()
            if ($health.IsSuccessStatusCode) { $ready = $true; break }
        } catch {}
    }
    if (-not $ready) { throw "OpenCode HTTP server did not become ready." }

    $paths = @("/question", "/permission", "/doc", "/openapi.json")
    $results = foreach ($path in $paths) {
        try {
            $response = $client.GetAsync("http://127.0.0.1:$Port$path").GetAwaiter().GetResult()
            $body = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
            [pscustomobject]@{
                path = $path
                status = [int]$response.StatusCode
                contentType = $response.Content.Headers.ContentType.MediaType
                bodyPreview = $body.Substring(0, [Math]::Min(300, $body.Length))
            }
        } catch {
            [pscustomobject]@{ path = $path; status = "error"; contentType = $null; bodyPreview = $_.Exception.Message }
        }
    }
    $results | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $resultPath -Encoding utf8
    $results
} finally {
    if ($null -ne $client) { $client.Dispose() }
    if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
}

Write-Host "Probe root: $probeRoot"
Write-Host "OpenCode executable: $OpenCodePath"
Get-Content -LiteralPath $resultPath
if (Test-Path -LiteralPath $stderrPath) { Get-Content -LiteralPath $stderrPath }
