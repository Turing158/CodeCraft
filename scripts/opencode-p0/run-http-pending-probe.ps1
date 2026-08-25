param(
    [string]$OpenCodePath,
    [ValidatePattern("^[A-Za-z0-9._-]+$")]
    [string]$ProbeName = "http-pending",
    [ValidateRange(1024, 65535)]
    [int]$OpenCodePort = 48770,
    [ValidateRange(1024, 65535)]
    [int]$ModelPort = 48769
)

$ErrorActionPreference = "Stop"

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Definition
$probeRoot = Join-Path $env:TEMP "codecraft-opencode-p0-$ProbeName"
$projectDir = Join-Path $probeRoot "project"
$configDir = Join-Path $probeRoot "config"
$dataDir = Join-Path $probeRoot "data"
$cacheDir = Join-Path $probeRoot "cache"
$stateDir = Join-Path $probeRoot "state"
$modelStdout = Join-Path $probeRoot "model.stdout.log"
$modelStderr = Join-Path $probeRoot "model.stderr.log"
$openCodeStdout = Join-Path $probeRoot "opencode.stdout.log"
$openCodeStderr = Join-Path $probeRoot "opencode.stderr.log"
$resultPath = Join-Path $probeRoot "pending-result.json"

New-Item -ItemType Directory -Force -Path $projectDir,$configDir,$dataDir,$cacheDir,$stateDir | Out-Null
Remove-Item -LiteralPath $modelStdout,$modelStderr,$openCodeStdout,$openCodeStderr,$resultPath -Force -ErrorAction SilentlyContinue

if ([string]::IsNullOrWhiteSpace($OpenCodePath)) {
    $OpenCodePath = "E:\nodejs\node_global\node_modules\opencode-ai\bin\opencode.exe"
    if (-not (Test-Path -LiteralPath $OpenCodePath)) {
        $OpenCodePath = (Get-Command opencode -ErrorAction Stop).Source
    }
}
$OpenCodePath = (Resolve-Path -LiteralPath $OpenCodePath -ErrorAction Stop).Path
$nodePath = (Get-Command node -ErrorAction Stop).Source

$config = @{
    formatter = $false
    lsp = $false
    provider = @{
        probe = @{
            name = "CodeCraft P0 Probe"
            id = "probe"
            env = @()
            npm = "@ai-sdk/openai-compatible"
            models = @{
                "probe-model" = @{
                    id = "probe-model"
                    name = "Probe Model"
                    attachment = $false
                    reasoning = $false
                    temperature = $false
                    tool_call = $true
                    release_date = "2026-08-19"
                    limit = @{ context = 100000; output = 10000 }
                    cost = @{ input = 0; output = 0 }
                    options = @{}
                }
            }
            options = @{ apiKey = "probe-key"; baseURL = "http://127.0.0.1:$ModelPort/v1" }
        }
    }
}

$env:OPENCODE_CONFIG_DIR = $configDir
$env:XDG_DATA_HOME = $dataDir
$env:XDG_CACHE_HOME = $cacheDir
$env:XDG_STATE_HOME = $stateDir
$env:OPENCODE_DISABLE_AUTOUPDATE = "true"
$env:OPENCODE_CONFIG_CONTENT = $config | ConvertTo-Json -Depth 12 -Compress

function Invoke-ProbeApi {
    param(
        [ValidateSet("Get", "Post")]
        [string]$Method,
        [string]$Path,
        [object]$Body
    )
    $headers = @{ "x-opencode-directory" = [Uri]::EscapeDataString($projectDir) }
    $input = @{ Uri = "http://127.0.0.1:$OpenCodePort$Path"; Method = $Method; Headers = $headers; TimeoutSec = 15 }
    if ($PSBoundParameters.ContainsKey("Body")) {
        $input.ContentType = "application/json"
        $input.Body = $Body | ConvertTo-Json -Depth 12 -Compress
    }
    return Invoke-RestMethod @input
}

function Wait-Pending {
    param([ValidateSet("question", "permission")][string]$Kind, [string]$SessionID)
    for ($attempt = 0; $attempt -lt 120; $attempt++) {
        $items = @(Invoke-ProbeApi -Method Get -Path "/$Kind")
        $match = $items | Where-Object { $_.sessionID -eq $SessionID } | Select-Object -First 1
        if ($null -ne $match) { return $match }
        Start-Sleep -Milliseconds 250
    }
    throw "Timed out waiting for $Kind request for session $SessionID."
}

function Wait-Released {
    param([ValidateSet("question", "permission")][string]$Kind, [string]$RequestID, [string]$SessionID)
    for ($attempt = 0; $attempt -lt 160; $attempt++) {
        $items = @(Invoke-ProbeApi -Method Get -Path "/$Kind")
        $pending = $items | Where-Object { $_.id -eq $RequestID }
        $statuses = Invoke-ProbeApi -Method Get -Path "/session/status"
        $entry = $statuses.PSObject.Properties[$SessionID]
        $busy = $null -ne $entry -and $entry.Value.type -ne "idle"
        if ($null -eq $pending -and -not $busy) { return }
        Start-Sleep -Milliseconds 250
    }
    throw "Timed out waiting for $Kind request $RequestID to release session $SessionID."
}

function Start-Scenario {
    param([string]$Name)
    $rules = @(
        @{ permission = "*"; pattern = "*"; action = "allow" },
        @{ permission = "bash"; pattern = "*"; action = "ask" },
        @{ permission = "external_directory"; pattern = "*"; action = "deny" }
    )
    $session = Invoke-ProbeApi -Method Post -Path "/session" -Body @{
        title = "CodeCraft P0 $Name"
        model = @{ id = "probe-model"; providerID = "probe" }
        permission = $rules
    }
    Invoke-ProbeApi -Method Post -Path "/session/$($session.id)/prompt_async" -Body @{
        model = @{ providerID = "probe"; modelID = "probe-model" }
        parts = @(@{ type = "text"; text = $Name })
    } | Out-Null
    return $session
}

$modelProcess = Start-Process -FilePath $nodePath -ArgumentList @((Join-Path $scriptRoot "fake-openai-server.js"), $ModelPort) -PassThru -NoNewWindow -WorkingDirectory $projectDir -RedirectStandardOutput $modelStdout -RedirectStandardError $modelStderr
$openCodeProcess = $null
try {
    $modelProcess.ProcessorAffinity = [IntPtr]3
    $modelReady = $false
    for ($attempt = 0; $attempt -lt 40; $attempt++) {
        Start-Sleep -Milliseconds 250
        try {
            $health = Invoke-RestMethod -Uri "http://127.0.0.1:$ModelPort/health" -TimeoutSec 2
            if ($health.ok) { $modelReady = $true; break }
        } catch {}
    }
    if (-not $modelReady) { throw "Fake model server did not become ready." }

    $openCodeProcess = Start-Process -FilePath $OpenCodePath -ArgumentList @("serve", "--hostname", "127.0.0.1", "--port", $OpenCodePort) -PassThru -NoNewWindow -WorkingDirectory $projectDir -RedirectStandardOutput $openCodeStdout -RedirectStandardError $openCodeStderr
    $openCodeProcess.ProcessorAffinity = [IntPtr]3
    $openCodeReady = $false
    for ($attempt = 0; $attempt -lt 60; $attempt++) {
        Start-Sleep -Milliseconds 250
        try {
            $health = Invoke-RestMethod -Uri "http://127.0.0.1:$OpenCodePort/global/health" -TimeoutSec 2
            if ($health) { $openCodeReady = $true; break }
        } catch {}
    }
    if (-not $openCodeReady) { throw "OpenCode server did not become ready." }

    $results = [ordered]@{}

    $session = Start-Scenario -Name "QUESTION_REPLY"
    $request = Wait-Pending -Kind question -SessionID $session.id
    $answers = [object[]]@(,[object[]]@("Yes"))
    $reply = Invoke-ProbeApi -Method Post -Path "/question/$($request.id)/reply" -Body @{ answers = $answers }
    Wait-Released -Kind question -RequestID $request.id -SessionID $session.id
    $results.questionReply = @{ requestID = $request.id; sessionID = $session.id; response = $reply; released = $true }

    $session = Start-Scenario -Name "QUESTION_REJECT"
    $request = Wait-Pending -Kind question -SessionID $session.id
    $reply = Invoke-ProbeApi -Method Post -Path "/question/$($request.id)/reject"
    Wait-Released -Kind question -RequestID $request.id -SessionID $session.id
    $results.questionReject = @{ requestID = $request.id; sessionID = $session.id; response = $reply; released = $true }

    foreach ($decision in @("once", "reject", "always")) {
        $name = "PERMISSION_$($decision.ToUpperInvariant())"
        $session = Start-Scenario -Name $name
        $request = Wait-Pending -Kind permission -SessionID $session.id
        $reply = Invoke-ProbeApi -Method Post -Path "/permission/$($request.id)/reply" -Body @{ reply = $decision }
        Wait-Released -Kind permission -RequestID $request.id -SessionID $session.id
        $results["permission$($decision.Substring(0,1).ToUpperInvariant())$($decision.Substring(1))"] = @{
            requestID = $request.id
            sessionID = $session.id
            response = $reply
            released = $true
        }
    }

    $output = [ordered]@{
        openCodeExecutable = $OpenCodePath
        openCodeVersion = (& $OpenCodePath --version).Trim()
        question = @{ list = $true; reply = $true; reject = $true }
        permission = @{ list = $true; once = $true; always = $true; reject = $true }
        results = $results
        compatible = $true
    }
    $output | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $resultPath -Encoding utf8
} finally {
    if ($null -ne $openCodeProcess -and -not $openCodeProcess.HasExited) { Stop-Process -Id $openCodeProcess.Id -Force }
    if (-not $modelProcess.HasExited) { Stop-Process -Id $modelProcess.Id -Force }
}

Write-Host "Probe root: $probeRoot"
Get-Content -LiteralPath $resultPath
if (Test-Path -LiteralPath $openCodeStderr) { Get-Content -LiteralPath $openCodeStderr }
if (Test-Path -LiteralPath $modelStderr) { Get-Content -LiteralPath $modelStderr }
