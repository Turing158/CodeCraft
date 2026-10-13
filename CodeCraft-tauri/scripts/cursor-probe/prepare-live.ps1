param(
    [string]$ProductExe = (Join-Path $PSScriptRoot '../../src-tauri/target/release/codecraft-tauri.exe'),
    [string]$CursorExe = 'D:/software/cursor/Cursor.exe',
    [switch]$Launch
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'limit-cpu.ps1')
$ProductExe = (Resolve-Path -LiteralPath $ProductExe).Path
$CursorExe = (Resolve-Path -LiteralPath $CursorExe).Path
$cursorRun = Join-Path $PSScriptRoot ('runs/v5-live-' + [guid]::NewGuid().ToString())
$cursorProject = Join-Path $cursorRun 'project'
foreach ($cursorDirectory in @('project/.cursor', 'home/.cursor', 'local', 'roaming', 'webview')) {
    New-Item -ItemType Directory -Path (Join-Path $cursorRun $cursorDirectory) -Force | Out-Null
}
$cursorRun = (Resolve-Path -LiteralPath $cursorRun).Path
Set-Content -LiteralPath (Join-Path $cursorProject 'sample.txt') -Value 'Synthetic Cursor v5 online acceptance fixture. No production data.' -Encoding utf8
& cargo run --locked --manifest-path (Join-Path $PSScriptRoot '../../src-tauri/cursor-core/Cargo.toml') --example prepare_live -- $cursorRun $ProductExe $CursorExe
if ($LASTEXITCODE -ne 0) { throw 'Isolated production setup failed' }
$cursorLocal = Join-Path $cursorRun 'local'
$cursorWrapper = Join-Path $cursorRun 'product-hook.cmd'
@('@echo off', 'setlocal', ('set "LOCALAPPDATA=' + $cursorLocal + '"'), ('"' + $ProductExe + '" %*'), 'exit /b %errorlevel%') |
    Set-Content -LiteralPath $cursorWrapper -Encoding ascii
$cursorHooks = Get-Content -LiteralPath (Join-Path $cursorRun 'home/.cursor/hooks.json') -Raw | ConvertFrom-Json
foreach ($cursorEvent in $cursorHooks.hooks.PSObject.Properties) {
    foreach ($cursorHandler in $cursorEvent.Value) { $cursorHandler.command = $cursorHandler.command.Replace($ProductExe, $cursorWrapper) }
}
$cursorHooks | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $cursorProject '.cursor/hooks.json') -Encoding utf8
@{mcpServers=@{codecraft_cursor=@{command=$ProductExe;args=@('--codecraft-cursor-mcp');env=@{LOCALAPPDATA=$cursorLocal}}}} |
    ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $cursorProject '.cursor/mcp.json') -Encoding utf8
@{enabled=$true;port=18787;bind='loopback';token=[guid]::NewGuid().ToString('N');allowApprovals=$true;auditRemote=$true} |
    ConvertTo-Json | Set-Content -LiteralPath (Join-Path $cursorLocal 'CodeCraft/lan-server.json') -Encoding utf8
@{mode='manual';minimalMode=$false} | ConvertTo-Json |
    Set-Content -LiteralPath (Join-Path $cursorLocal 'CodeCraft/approval-settings.json') -Encoding utf8
$cursorMetadata = @{root=$cursorRun;exe=$ProductExe;cursorExe=$CursorExe;exeSha256=(Get-FileHash -LiteralPath $ProductExe -Algorithm SHA256).Hash;cursorVersion=(Get-Item -LiteralPath $CursorExe).VersionInfo.ProductVersion}
$cursorMetadata | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $cursorRun 'run.json') -Encoding utf8
$cursorMetadata | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $PSScriptRoot 'runs/v5-live-current.json') -Encoding utf8
if ($Launch) {
    $cursorEnvironment = @{}
    foreach ($cursorName in @('USERPROFILE','LOCALAPPDATA','APPDATA','WEBVIEW2_USER_DATA_FOLDER')) { $cursorEnvironment[$cursorName] = [Environment]::GetEnvironmentVariable($cursorName, 'Process') }
    try {
        $env:USERPROFILE = Join-Path $cursorRun 'home'
        $env:LOCALAPPDATA = $cursorLocal
        $env:APPDATA = Join-Path $cursorRun 'roaming'
        $env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $cursorRun 'webview'
        $cursorProduct = Start-Process -FilePath $ProductExe -WorkingDirectory $cursorProject -PassThru -WindowStyle Hidden -RedirectStandardError (Join-Path $cursorRun 'product-error.log')
        $cursorProduct.ProcessorAffinity = [IntPtr]$cursorProbeMask
        $cursorProduct.Id | Set-Content -LiteralPath (Join-Path $cursorRun 'product.pid')
    } finally {
        foreach ($cursorName in $cursorEnvironment.Keys) { [Environment]::SetEnvironmentVariable($cursorName, $cursorEnvironment[$cursorName], 'Process') }
    }
    Start-Process -FilePath $CursorExe -ArgumentList @('--reuse-window', ('"' + $cursorProject + '"')) -WindowStyle Hidden
}
Write-Output "Prepared isolated live run: $cursorRun"
Write-Output 'No runtime verdict recorded. Use the real Cursor client and CodeCraft UI to perform acceptance.'
