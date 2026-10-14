param(
    [Parameter(Mandatory=$true)][string]$Assets,
    [Parameter(Mandatory=$true)][string]$Version
)
$ErrorActionPreference = 'Stop'
# This installs/uninstalls software. Never run it against a developer's real profile.
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'Run only on a disposable GitHub Actions Windows runner. Use the manual acceptance checklist on a clean VM.' }
if ($Version -notmatch '^\d+\.\d+\.\d+-(alpha|beta|rc)\.\d+$') { throw 'Invalid pre-release version' }
$assetRoot = (Resolve-Path -LiteralPath $Assets).Path
$evidence = Join-Path (Get-Location).Path 'dist\installer-smoke'
New-Item -ItemType Directory -Path $evidence -Force | Out-Null
$stateRoot = Join-Path $env:USERPROFILE '.taskpulse'
if (Test-Path -LiteralPath $stateRoot) { throw 'Expected a fresh TaskPulse profile; refusing to overwrite existing data' }
$workspace = Join-Path $env:TEMP ('TaskPulse-installer-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $workspace | Out-Null
$marker = Join-Path $stateRoot 'installer-smoke-marker.txt'
New-Item -ItemType Directory -Path $stateRoot | Out-Null
Set-Content -LiteralPath $marker -Value 'preserve-user-data' -Encoding UTF8
$initialPath = $env:PATH
$initialWebViewArgs = $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
$process = $null
$results = @()

function Invoke-Installer([string]$File, [string]$Arguments) {
    $child = Start-Process -FilePath $File -ArgumentList $Arguments -WindowStyle Hidden -Wait -PassThru
    if ($child.ExitCode -notin @(0,3010)) { throw "Installer exit $($child.ExitCode): $File" }
}
function Stop-OwnedApp([string]$Exe) {
    if ($null -ne $script:process -and -not $script:process.HasExited) {
        $live = Get-Process -Id $script:process.Id -ErrorAction Stop
        if ([IO.Path]::GetFullPath($live.Path) -ne [IO.Path]::GetFullPath($Exe)) { throw 'Refusing to stop a process with a different executable path' }
        Stop-Process -Id $live.Id
        $script:process.WaitForExit(10000) | Out-Null
    }
    $script:process = $null
}
function Get-AppPage {
    for ($attempt=0; $attempt -lt 100; $attempt++) {
        try {
            $pages = Invoke-RestMethod 'http://127.0.0.1:19347/json' -TimeoutSec 2
            $page = $pages | Where-Object { $_.url -match 'tauri.localhost' -and $_.url -notmatch '/(notice|manage)' } | Select-Object -First 1
            if ($page) { return $page }
        } catch { }
        if ($script:process.HasExited) { throw 'Installed app exited before its native WebView was ready' }
        Start-Sleep -Milliseconds 200
    }
    throw 'Installed app did not expose its embedded frontend'
}
function Test-NativeIPC([string]$Url) {
    $socket = New-Object System.Net.WebSockets.ClientWebSocket
    $timeout = New-Object System.Threading.CancellationTokenSource(15000)
    try {
        $socket.ConnectAsync([Uri]$Url,$timeout.Token).GetAwaiter().GetResult()
        $expression = '(async()=>{for(let i=0;i<100;i++){if(document.querySelector(".dashboard")&&globalThis.__TAURI_INTERNALS__)break;await new Promise(r=>setTimeout(r,100));}const invoke=globalThis.__TAURI_INTERNALS__.invoke;const s=await invoke("get_monitoring_settings");const tasks=await invoke("get_agents");const prompts=await invoke("get_pending_task_prompts");return {embedded:!!document.querySelector(".dashboard"),onboarding:s.onboarding_complete,tasks:tasks.length,prompts:prompts.length};})()'
        $request = @{id=1;method='Runtime.evaluate';params=@{expression=$expression;awaitPromise=$true;returnByValue=$true}} | ConvertTo-Json -Depth 10 -Compress
        $bytes = [Text.Encoding]::UTF8.GetBytes($request)
        $segment = New-Object 'System.ArraySegment[byte]' -ArgumentList (,$bytes)
        $socket.SendAsync($segment,[Net.WebSockets.WebSocketMessageType]::Text,$true,$timeout.Token).GetAwaiter().GetResult()
        while ($true) {
            $buffer = New-Object byte[] 65536
            $incoming = New-Object 'System.ArraySegment[byte]' -ArgumentList (,$buffer)
            $message = New-Object Text.StringBuilder
            do {
                $received = $socket.ReceiveAsync($incoming,$timeout.Token).GetAwaiter().GetResult()
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
    $manifest = Get-Content -Raw -Encoding UTF8 (Join-Path $assetRoot 'release-manifest.json') | ConvertFrom-Json
    if ($manifest.version -ne $Version -or $manifest.commit -notmatch '^[a-f0-9]{40}$') { throw 'Release metadata mismatch' }
    foreach ($line in Get-Content -Encoding UTF8 (Join-Path $assetRoot 'SHA256SUMS.txt')) {
        if ($line -notmatch '^([a-f0-9]{64})  ([A-Za-z0-9_.-]+)$') { throw 'Invalid checksum path' }
        if ((Get-FileHash -LiteralPath (Join-Path $assetRoot $Matches[2]) -Algorithm SHA256).Hash.ToLowerInvariant() -ne $Matches[1]) { throw 'Installer checksum mismatch' }
    }
    foreach ($kind in @('nsis','msi')) {
        $installDir = Join-Path $workspace $kind
        New-Item -ItemType Directory -Path $installDir | Out-Null
        $exe = Join-Path $installDir 'agent-tray.exe'
        $package = if ($kind -eq 'nsis') { Join-Path $assetRoot "TaskPulse_${Version}_windows_x64_setup.exe" } else { Join-Path $assetRoot "TaskPulse_${Version}_windows_x64.msi" }
        $arguments = if ($kind -eq 'nsis') { "/S /D=$installDir" } else { "/i `"$package`" /qn /norestart INSTALLDIR=`"$installDir`" /L*v `"$(Join-Path $evidence 'msi-install.log')`"" }
        $installer = if ($kind -eq 'nsis') { $package } else { Join-Path $env:SystemRoot 'System32\msiexec.exe' }
        Invoke-Installer $installer $arguments
        if (-not (Test-Path -LiteralPath $exe)) { throw "Missing installed executable: $exe" }
        foreach ($resource in @('agent-tray-hook.ps1','install-hooks.ps1')) {
            if (-not (Test-Path -LiteralPath (Join-Path $installDir "scripts\hooks\$resource"))) { throw "Missing packaged Hook resource: $resource" }
        }
        # Child runtime cannot find Rust, Bun, Node or repository build tools.
        $env:PATH = "$env:SystemRoot\System32;$env:SystemRoot"
        $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=19347'
        $versionOutput = Join-Path $evidence "$kind-version.txt"
        $versionProcess = Start-Process -FilePath $exe -ArgumentList '--version' -WindowStyle Hidden -Wait -PassThru -RedirectStandardOutput $versionOutput
        if ($versionProcess.ExitCode -ne 0 -or (Get-Content -Raw $versionOutput).Trim() -ne "TaskPulse $Version") { throw 'Installed executable version differs from release' }
        $process = Start-Process -FilePath $exe -ArgumentList '--show' -WindowStyle Hidden -PassThru
        $ipc = Test-NativeIPC (Get-AppPage).webSocketDebuggerUrl
        Stop-OwnedApp $exe
        # Restart must retain onboarding/data. Reinstall is NOT a cross-version upgrade test.
        $process = Start-Process -FilePath $exe -ArgumentList '--show' -WindowStyle Hidden -PassThru
        $null = Test-NativeIPC (Get-AppPage).webSocketDebuggerUrl
        Stop-OwnedApp $exe
        Invoke-Installer $installer $arguments
        if (-not (Test-Path -LiteralPath $marker)) { throw 'Reinstall removed user data' }
        $signature = (Get-AuthenticodeSignature -FilePath $package).Status.ToString()
        if ($kind -eq 'nsis') {
            $uninstaller = (Resolve-Path -LiteralPath (Join-Path $installDir 'uninstall.exe')).Path
            if (-not $uninstaller.StartsWith($workspace+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) { throw 'Uninstaller escaped smoke workspace' }
            Invoke-Installer $uninstaller '/S'
        } else {
            Invoke-Installer $installer "/x `"$package`" /qn /norestart /L*v `"$(Join-Path $evidence 'msi-uninstall.log')`""
        }
        for($n=0;$n -lt 50 -and (Test-Path -LiteralPath $exe);$n++){Start-Sleep -Milliseconds 200}
        if (Test-Path -LiteralPath $exe) { throw 'Uninstall left the application executable installed' }
        if (-not (Test-Path -LiteralPath $marker)) { throw 'Uninstall removed user state without permission' }
        $results += @{kind=$kind;version=$Version;signature=$signature;native_ipc=$ipc;restart=$true;reinstall=$true;uninstall=$true;cross_version_upgrade='not_verified';focus='not_verified'}
    }
} finally {
    if ($process -and -not $process.HasExited) { Stop-OwnedApp $process.Path }
    $env:PATH = $initialPath
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = $initialWebViewArgs
    $results | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $evidence 'result.json') -Encoding UTF8
}
