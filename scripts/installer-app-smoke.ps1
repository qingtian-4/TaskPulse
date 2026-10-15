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
function Test-NativeIPC([string]$Url) {
    $socket = New-Object System.Net.WebSockets.ClientWebSocket
    $timeout = New-Object System.Threading.CancellationTokenSource(15000)
    try {
        [void]$socket.ConnectAsync([Uri]$Url,$timeout.Token).GetAwaiter().GetResult()
        $expression = '(async()=>{for(let i=0;i<100;i++){if(document.querySelector(".dashboard")&&globalThis.__TAURI_INTERNALS__)break;await new Promise(r=>setTimeout(r,100));}const invoke=globalThis.__TAURI_INTERNALS__.invoke;const s=await invoke("get_monitoring_settings");const tasks=await invoke("get_agents");const prompts=await invoke("get_pending_task_prompts");return {embedded:!!document.querySelector(".dashboard"),onboarding:s.onboarding_complete,tasks:tasks.length,prompts:prompts.length};})()'
        $request = @{id=1;method='Runtime.evaluate';params=@{expression=$expression;awaitPromise=$true;returnByValue=$true}} | ConvertTo-Json -Depth 10 -Compress
        $bytes = [Text.Encoding]::UTF8.GetBytes($request)
        $segment = New-Object 'System.ArraySegment[byte]' -ArgumentList (,$bytes)
        [void]$socket.SendAsync($segment,[Net.WebSockets.WebSocketMessageType]::Text,$true,$timeout.Token).GetAwaiter().GetResult()
        while ($true) {
            $buffer = New-Object byte[] 65536
            $incoming = New-Object 'System.ArraySegment[byte]' -ArgumentList (,$buffer)
            $message = New-Object Text.StringBuilder
            do {
                $received = $socket.ReceiveAsync($incoming,$timeout.Token).GetAwaiter().GetResult()
                if ($received.MessageType -eq [Net.WebSockets.WebSocketMessageType]::Close) { throw 'Native CDP connection closed' }
                [void]$message.Append([Text.Encoding]::UTF8.GetString($buffer,0,$received.Count))
            } while (-not $received.EndOfMessage)
            $reply = $message.ToString() | ConvertFrom-Json
            if ($reply.id -ne 1) { continue }
            if ($reply.error -or $reply.result.exceptionDetails) { throw 'Native WebView IPC failed' }
            $value = $reply.result.result.value
            if (-not $value.embedded -or $value.onboarding -or $value.tasks -ne 0 -or $value.prompts -ne 0) { throw 'First launch imported history, enabled monitoring, or failed to render onboarding' }
            return $value
        }
    } finally { $socket.Dispose(); $timeout.Dispose() }
}

try {
    foreach ($launch in @('first-launch','restart')) {
        $phase = $launch
        Write-Host "[$Kind] medium-integrity $launch"
        $process = Start-Process -FilePath $Exe -ArgumentList '--show' -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $Evidence "$Kind-$launch.stdout.log") -RedirectStandardError (Join-Path $Evidence "$Kind-$launch.stderr.log")
        $value = Test-NativeIPC (Get-AppPage).webSocketDebuggerUrl
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
        @{kind=$Kind;integrity='medium';phase=$phase;error=$failure;native_ipc=$ipc;restart=($phase -eq 'passed');pages=@($lastPages);last_probe_error=$lastProbeError;expected_exe=$Exe;observed_exe=$observedExe} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $Evidence "$Kind-native.json") -Encoding UTF8
    }
}
