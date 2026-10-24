param(
    [Parameter(Mandatory=$true)][string]$Exe,
    [Parameter(Mandatory=$true)][string]$Evidence,
    [Parameter(Mandatory=$true)][string]$Kind
)
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'Disposable CI runner only' }
$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Run application acceptance at medium integrity, not elevated' }
. (Join-Path $PSScriptRoot 'installer-process.ps1')
. (Join-Path $PSScriptRoot 'installer-webview.ps1')
$Exe = (Resolve-Path -LiteralPath $Exe).Path
$Evidence = (Resolve-Path -LiteralPath $Evidence).Path
$env:PATH = "$env:SystemRoot\System32;$env:SystemRoot"
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=19347'
$env:RUST_LOG = 'info'
$process = $null
$phase = 'prepare'
$failure = $null
$ipc = $null
$lastPages = @()
$lastProbeError = $null
$observedExe = $null
$launchEvidence = New-Object 'Collections.Generic.List[object]'

function Stop-OwnedApp {
    if ($script:process -and -not $script:process.HasExited) {
        $live = Get-Process -Id $script:process.Id -ErrorAction Stop
        $script:observedExe = $live.Path
        if (-not (Test-InstallerSameExecutable $live.Path $Exe)) { throw "Refusing to stop an unrelated executable: expected $Exe; observed $($live.Path)" }
        Stop-Process -Id $live.Id
        if (-not $script:process.WaitForExit(10000)) { throw 'Installed app did not stop' }
    }
    $script:process = $null
    # The browser child can outlive its host briefly; never attach the next
    # launch to a previous CDP target during restart acceptance.
    $closed = $false
    $closeClock = [Diagnostics.Stopwatch]::StartNew()
    while ($closeClock.Elapsed.TotalSeconds -lt 20) {
        try { $null = Invoke-RestMethod 'http://127.0.0.1:19347/json' -TimeoutSec 1 }
        catch { $closed = $true; break }
        Start-Sleep -Milliseconds 100
    }
    if (-not $closed) { throw 'Previous native CDP endpoint is still active after app stop' }
}
function Get-AppPage {
    $clock = [Diagnostics.Stopwatch]::StartNew()
    while ($clock.Elapsed.TotalSeconds -lt 45) {
        try {
            $pages = Invoke-RestMethod 'http://127.0.0.1:19347/json' -TimeoutSec 1
            $script:lastProbeError = $null
            $script:lastPages = @($pages | Select-Object type,url)
            $page = $pages | Where-Object { $_.url -match '^https?://tauri\.localhost(?:/|$)' -and $_.url -notmatch '/(notice|manage)' } | Select-Object -First 1
            if ($page) { return $page }
        } catch { $script:lastProbeError = $_.Exception.Message }
        if ($script:process.HasExited) { throw "Installed app exited: $($script:process.ExitCode)" }
        Start-Sleep -Milliseconds 200
    }
    throw 'Installed app did not expose its embedded frontend within 45 seconds; see startup logs and native evidence'
}

try {
    foreach ($launch in @('first-launch','restart')) {
        $phase = $launch
        Write-Host "[$Kind] medium-integrity $launch"
        $process = Start-Process -FilePath $Exe -ArgumentList '--show' -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $Evidence "$Kind-$launch.stdout.log") -RedirectStandardError (Join-Path $Evidence "$Kind-$launch.stderr.log")
        $trace = New-Object 'Collections.Generic.List[object]'
        $launchEvidence.Add(@{launch=$launch;probe=$trace})
        $value = Invoke-InstallerWebViewProbe -Url (Get-AppPage).webSocketDebuggerUrl -Trace $trace
        if ($launch -eq 'first-launch') { $ipc = $value }
        Stop-OwnedApp
    }
    $phase = 'passed'
} catch {
    $failure = $_.Exception.Message
    throw
} finally {
    try { Stop-OwnedApp } finally {
        # Fresh CI profile only; no task prompts/commands or credential material.
        @{kind=$Kind;integrity='medium';phase=$phase;error=$failure;native_ipc=$ipc;restart=($phase -eq 'passed');probe_commit=$env:TASKPULSE_PROBE_COMMIT;launch_evidence=$launchEvidence;pages=@($lastPages);last_probe_error=$lastProbeError;expected_exe=$Exe;observed_exe=$observedExe} | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $Evidence "$Kind-native.json") -Encoding UTF8
    }
}
