# Interactive test-owned window only; no user windows, hooks or settings.
param([Parameter(Mandatory=$true)][string]$StatePath,
      [Parameter(Mandatory=$true)][string]$ControlPath)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class TaskPulseFocusFixture {
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
}
'@
$form = New-Object Windows.Forms.Form
$form.Text = 'TaskPulse automatic focus acceptance - closes automatically'
$form.Width = 360
$form.Height = 160
$form.StartPosition = 'CenterScreen'
$window = $form.Handle
[uint32]$owner = 0
$thread = [TaskPulseFocusFixture]::GetWindowThreadProcessId($window,[ref]$owner)
if ($owner -ne $PID -or $thread -eq 0) { throw 'Fixture window ownership mismatch' }
$creation = [Diagnostics.Process]::GetCurrentProcess().StartTime.ToUniversalTime().ToFileTimeUtc()
# windowsHide suppresses PowerShell's console and supplies a hidden startup
# show command. Explicitly show only our form after WinForms' initial Show.
$form.Add_Shown({ [void][TaskPulseFocusFixture]::ShowWindow($form.Handle,5) })
$form.Add_FormClosing({ param($sender,$eventArgs) [Console]::Error.WriteLine('Fixture closing: '+$eventArgs.CloseReason) })
$script:lastRequest = 0
$timer = New-Object Windows.Forms.Timer
$timer.Interval = 50
$timer.Add_Tick({
    if (Test-Path -LiteralPath $ControlPath) {
        # A partial test-control write is retried on the next UI tick.
        $request = $null
        try { $request = [IO.File]::ReadAllText($ControlPath) | ConvertFrom-Json } catch {}
        if ($request -and $request.id -gt $script:lastRequest) {
            if ($request.action -ne 'minimize') { throw 'Unknown fixture action' }
            $form.WindowState = [Windows.Forms.FormWindowState]::Minimized
            $script:lastRequest = $request.id
        }
    }
    [uint32]$liveOwner = 0
    $liveThread = [TaskPulseFocusFixture]::GetWindowThreadProcessId($window,[ref]$liveOwner)
    $state = @{pid=$PID;identity="${creation}:$($window.ToInt64()):${thread}";hwnd=$window.ToInt64();
        visible=[TaskPulseFocusFixture]::IsWindowVisible($window);
        identity_matches=($liveOwner -eq $PID -and $liveThread -eq $thread -and $form.Handle -eq $window);
        foreground=([TaskPulseFocusFixture]::GetForegroundWindow() -eq $window);
        minimized=[TaskPulseFocusFixture]::IsIconic($window);request_id=$script:lastRequest}
    [IO.File]::WriteAllText($StatePath,($state | ConvertTo-Json -Compress))
})
try {
    $timer.Start()
    [Windows.Forms.Application]::Run($form)
} finally {
    $timer.Stop()
    $timer.Dispose()
    $form.Dispose()
}
