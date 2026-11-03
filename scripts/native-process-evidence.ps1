param([Parameter(Mandatory=$true)][int]$ProcessId,[Parameter(Mandatory=$true)][string]$Exe,[int]$DebugPort,[switch]$ChildrenOnly)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$clock = [Diagnostics.Stopwatch]::StartNew()
function Write-EvidenceStage([string]$Stage) {
    [Console]::Error.WriteLine(("TaskPulseProbe stage={0} elapsed_ms={1}" -f $Stage,$clock.ElapsedMilliseconds))
}
Write-EvidenceStage 'script-entry'
# The sibling path is already absolute. Avoid PowerShell provider resolution
# here, which stalled before Get-Process on fresh hosted standard accounts.
$installerPaths = [IO.Path]::Combine($PSScriptRoot, 'installer-process.ps1')
Write-EvidenceStage 'load-installer-paths'
. $installerPaths
Write-EvidenceStage 'get-process'
# Query this process instance directly. Cmdlet discovery can cold-load the
# Management module before Get-Process reaches the process query.
$owned=[Diagnostics.Process]::GetProcessById($ProcessId)
try {
    Write-EvidenceStage 'open-process-handle'
    $handle=$owned.Handle
    Write-EvidenceStage 'check-process-exit'
    if ($owned.HasExited) { throw 'Native evidence process ownership mismatch' }
    Write-EvidenceStage 'read-process-path'
    $actualPath=$owned.MainModule.FileName
    if (-not $actualPath -or -not $Exe) { throw 'Native evidence process ownership mismatch' }
    Write-EvidenceStage 'resolve-add-type'
    $addTypeCommand=$ExecutionContext.InvokeCommand.GetCommand('Add-Type',[Management.Automation.CommandTypes]::Cmdlet)
    if ($null -eq $addTypeCommand) { throw 'Native evidence compiler command unavailable' }
    Write-EvidenceStage 'resolved-add-type'
    Write-EvidenceStage 'compile-installer-paths'
    Initialize-InstallerPaths
    Write-EvidenceStage 'compare-final-paths'
    if (-not (Test-InstallerSameExecutable $actualPath $Exe)) { throw 'Native evidence process ownership mismatch' }
    Write-EvidenceStage 'verified-executable'
    if ($ChildrenOnly) {
        $children=@(Get-CimInstance Win32_Process -Filter "ParentProcessId = $ProcessId" | Select-Object -First 16 | ForEach-Object {
            $version=$null
            if ($_.ExecutablePath) { try {$version=[Diagnostics.FileVersionInfo]::GetVersionInfo($_.ExecutablePath).FileVersion} catch {} }
            # Never export command lines; retain only the relevant browser switch.
            $debug=if($_.CommandLine){$DebugPort -gt 0 -and $_.CommandLine -match ('--remote-debugging-port(?:=|\s+)"?'+$DebugPort+'(?:\D|$)')}else{$null}
            [pscustomobject]@{pid=$_.ProcessId;name=$_.Name;file_version=$version;requested_debug_port=$debug}
        })
        Write-EvidenceStage 'queried-children'
        [pscustomobject]@{children=$children} | ConvertTo-Json -Depth 5 -Compress
        return
    }
    Write-EvidenceStage 'compile-token-probe'
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Security.Principal;
public static class TaskPulseTokenEvidence {
    [DllImport("advapi32.dll",SetLastError=true)] static extern bool OpenProcessToken(IntPtr process,uint access,out IntPtr token);
    [DllImport("advapi32.dll",SetLastError=true)] static extern bool GetTokenInformation(IntPtr token,int kind,out int value,int size,out int needed);
    [DllImport("advapi32.dll",EntryPoint="GetTokenInformation",SetLastError=true)] static extern bool GetTokenBuffer(IntPtr token,int kind,IntPtr value,int size,out int needed);
    [DllImport("advapi32.dll")] static extern IntPtr GetSidSubAuthorityCount(IntPtr sid);
    [DllImport("advapi32.dll")] static extern IntPtr GetSidSubAuthority(IntPtr sid,uint index);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    public static int Read(IntPtr process,int kind) {
        IntPtr token;
        if(!OpenProcessToken(process,8,out token))throw new Win32Exception(Marshal.GetLastWin32Error());
        try {int value,needed;if(!GetTokenInformation(token,kind,out value,4,out needed))throw new Win32Exception(Marshal.GetLastWin32Error());return value;}
        finally {CloseHandle(token);}
    }
    public static int Integrity(IntPtr process) {
        IntPtr token;
        if(!OpenProcessToken(process,8,out token))throw new Win32Exception(Marshal.GetLastWin32Error());
        IntPtr buffer=IntPtr.Zero;
        try {
            int needed;GetTokenBuffer(token,25,IntPtr.Zero,0,out needed);
            if(needed<=0||needed>65536)throw new Exception("Invalid token evidence size");
            buffer=Marshal.AllocHGlobal(needed);
            if(!GetTokenBuffer(token,25,buffer,needed,out needed))throw new Win32Exception(Marshal.GetLastWin32Error());
            var sid=Marshal.ReadIntPtr(buffer);var count=Marshal.ReadByte(GetSidSubAuthorityCount(sid));
            if(count==0)throw new Exception("Invalid token integrity SID");
            return Marshal.ReadInt32(GetSidSubAuthority(sid,(uint)(count-1)));
        } finally {if(buffer!=IntPtr.Zero)Marshal.FreeHGlobal(buffer);CloseHandle(token);}
    }
    public static bool UserMatches(IntPtr process,string expected) {
        IntPtr token;
        if(!OpenProcessToken(process,8,out token))throw new Win32Exception(Marshal.GetLastWin32Error());
        IntPtr buffer=IntPtr.Zero;
        try {
            int needed;GetTokenBuffer(token,1,IntPtr.Zero,0,out needed);
            if(needed<=0||needed>65536)throw new Exception("Invalid user evidence size");
            buffer=Marshal.AllocHGlobal(needed);
            if(!GetTokenBuffer(token,1,buffer,needed,out needed))throw new Win32Exception(Marshal.GetLastWin32Error());
            return new SecurityIdentifier(Marshal.ReadIntPtr(buffer)).Value==expected;
        } finally {if(buffer!=IntPtr.Zero)Marshal.FreeHGlobal(buffer);CloseHandle(token);}
    }
}
'@
    Write-EvidenceStage 'loaded-token-probe'
    Write-EvidenceStage 'read-window-title'
    $title=[string]$owned.MainWindowTitle
    Write-EvidenceStage 'read-token-evidence'
    [pscustomobject]@{pid=$ProcessId;owned_executable=$true;user_matches_caller=[TaskPulseTokenEvidence]::UserMatches($handle,[Security.Principal.WindowsIdentity]::GetCurrent().User.Value);elevated=([TaskPulseTokenEvidence]::Read($handle,20) -ne 0);elevation_type=[TaskPulseTokenEvidence]::Read($handle,18);integrity_rid=[TaskPulseTokenEvidence]::Integrity($handle);main_window_title=$title.Substring(0,[Math]::Min(256,$title.Length))} | ConvertTo-Json -Depth 5 -Compress
    Write-EvidenceStage 'wrote-identity'
} finally {$owned.Dispose()}
