# Read-only native IPC probe; importing this file never installs or launches software.
function Get-InstallerProbeException($Details) {
    $description = [string]$Details.exception.description
    if (-not $description) { $description = [string]$Details.exception.value }
    $text = [string]$Details.text
    return @{text=$text.Substring(0,[Math]::Min(256,$text.Length));description=$description.Substring(0,[Math]::Min(1024,$description.Length))}
}

function Write-InstallerProbeStage([string]$Stage, [Diagnostics.Stopwatch]$Clock) {
    [Console]::Error.WriteLine(("TaskPulseProbe stage={0} elapsed_ms={1}" -f $Stage,$Clock.ElapsedMilliseconds))
}

function Invoke-InstallerCDPEvaluate($Socket, $Token, [string]$Expression, [int]$Id, [string]$Stage, [Diagnostics.Stopwatch]$Clock) {
    $request = @{id=$Id;method='Runtime.evaluate';params=@{expression=$Expression;awaitPromise=$true;returnByValue=$true}} | ConvertTo-Json -Depth 10 -Compress
    $bytes = [Text.Encoding]::UTF8.GetBytes($request)
    $segment = New-Object 'System.ArraySegment[byte]' -ArgumentList (,$bytes)
    Write-InstallerProbeStage "${Stage}.send" $Clock
    [void]$Socket.SendAsync($segment,[Net.WebSockets.WebSocketMessageType]::Text,$true,$Token).GetAwaiter().GetResult()
    while ($true) {
        $buffer = New-Object byte[] 65536
        $incoming = New-Object 'System.ArraySegment[byte]' -ArgumentList (,$buffer)
        $message = New-Object IO.MemoryStream
        try {
            do {
                $received = $Socket.ReceiveAsync($incoming,$Token).GetAwaiter().GetResult()
                if ($received.MessageType -eq [Net.WebSockets.WebSocketMessageType]::Close) { throw 'Native CDP connection closed' }
                if ($message.Length + $received.Count -gt 65536) { throw 'Native CDP reply exceeds probe budget' }
                $message.Write($buffer,0,$received.Count)
            } while (-not $received.EndOfMessage)
            $reply = [Text.Encoding]::UTF8.GetString($message.ToArray()) | ConvertFrom-Json
        } finally { $message.Dispose() }
        if ($reply.id -eq $Id) { Write-InstallerProbeStage "${Stage}.response" $Clock; return $reply }
    }
}

function Invoke-InstallerWebViewProbe {
    param([string]$Url, [AllowEmptyCollection()][Collections.Generic.List[object]]$Trace, [int]$TimeoutMilliseconds=15000)
    $socket = New-Object System.Net.WebSockets.ClientWebSocket
    $timeout = New-Object System.Threading.CancellationTokenSource($TimeoutMilliseconds)
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $id = 0
    $stage = 'connect'
    try {
        Write-InstallerProbeStage 'connect.start' $clock
        [void]$socket.ConnectAsync([Uri]$Url,$timeout.Token).GetAwaiter().GetResult()
        Write-InstallerProbeStage 'connect.connected' $clock
        # A matching target URL can precede the navigation commit. Never keep an
        # async readiness promise in the context that is about to be destroyed.
        $stage = 'readiness'
        $ready = $false
        while ($clock.ElapsedMilliseconds -lt $TimeoutMilliseconds) {
            $id++
            $reply = Invoke-InstallerCDPEvaluate $socket $timeout.Token '({ready:document.readyState!=="loading"&&!!document.querySelector(".dashboard")&&typeof globalThis.__TAURI_INTERNALS__?.invoke==="function",documentReady:document.readyState,hasDashboard:!!document.querySelector(".dashboard"),hasTauri:typeof globalThis.__TAURI_INTERNALS__?.invoke==="function"})' $id $stage $clock
            $entry = @{stage=$stage;id=$id;elapsed_ms=$clock.ElapsedMilliseconds}
            if ($reply.error) {
                $entry.protocol_error = @{code=$reply.error.code;message=([string]$reply.error.message).Substring(0,[Math]::Min(512,([string]$reply.error.message).Length))}
                $Trace.Add($entry)
                # Only the naturally reproduced navigation failure is waitable,
                # and only before any application IPC has been attempted.
                if ($reply.error.code -ne -32000 -or $reply.error.message -ne 'Execution context was destroyed.') { throw 'Native CDP readiness failed' }
            } elseif ($reply.result.exceptionDetails) {
                $entry.js_exception = Get-InstallerProbeException $reply.result.exceptionDetails
                $Trace.Add($entry); throw 'Native readiness JavaScript failed'
            } else {
                $value = $reply.result.result.value
                if ($null -eq $value.ready -or $null -eq $value.documentReady -or $null -eq $value.hasDashboard -or $null -eq $value.hasTauri) { throw 'Malformed native readiness reply' }
                $entry.readiness = $value; $Trace.Add($entry)
                if ($value.ready -eq $true -and $value.documentReady -in @('interactive','complete') -and $value.hasDashboard -eq $true -and $value.hasTauri -eq $true) { $ready=$true; break }
            }
            Start-Sleep -Milliseconds 100
        }
        if (-not $ready) { throw 'Native frontend readiness timed out' }
        $stage = 'ipc'; $id++
        $reply = Invoke-InstallerCDPEvaluate $socket $timeout.Token '(async()=>{const invoke=globalThis.__TAURI_INTERNALS__.invoke;const s=await invoke("get_monitoring_settings");const tasks=await invoke("get_agents");const prompts=await invoke("get_pending_task_prompts");return {embedded:!!document.querySelector(".dashboard"),onboarding:s.onboarding_complete,tasks:tasks.length,prompts:prompts.length};})()' $id $stage $clock
        $entry = @{stage=$stage;id=$id;elapsed_ms=$clock.ElapsedMilliseconds}
        if ($reply.error) {
            $entry.protocol_error = @{code=$reply.error.code;message=([string]$reply.error.message).Substring(0,[Math]::Min(512,([string]$reply.error.message).Length))}
            $Trace.Add($entry); throw 'Native CDP failed during IPC; not retried'
        }
        if ($reply.result.exceptionDetails) {
            $entry.js_exception = Get-InstallerProbeException $reply.result.exceptionDetails
            $Trace.Add($entry); throw 'Native WebView IPC JavaScript failed; not retried'
        }
        $value = $reply.result.result.value
        if ($value.embedded -isnot [bool] -or $value.onboarding -isnot [bool] -or $value.embedded -ne $true -or $value.onboarding -ne $false -or $value.tasks -ne 0 -or $value.prompts -ne 0) { throw 'First launch imported history, enabled monitoring, or failed to render onboarding' }
        $entry.validated=$true; $Trace.Add($entry)
        return $value
    } catch {
        if ($stage -eq 'readiness' -and $clock.ElapsedMilliseconds -ge $TimeoutMilliseconds) {
            $Trace.Add(@{stage=$stage;elapsed_ms=$clock.ElapsedMilliseconds;failure='Native frontend readiness timed out'})
            throw 'Native frontend readiness timed out'
        }
        $Trace.Add(@{stage=$stage;elapsed_ms=$clock.ElapsedMilliseconds;failure=$_.Exception.Message})
        throw
    } finally { Write-InstallerProbeStage "${stage}.finally" $clock; $socket.Dispose(); Write-InstallerProbeStage "${stage}.socket_disposed" $clock; $timeout.Dispose() }
}
