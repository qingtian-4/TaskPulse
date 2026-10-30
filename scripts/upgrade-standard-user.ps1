param(
    [Parameter(Mandatory=$true)][string]$Plan,
    [Parameter(Mandatory=$true)][string]$Python
)
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') { throw 'Disposable hosted CI only' }
$planPath = (Resolve-Path -LiteralPath $Plan).Path
$info = Get-Content -Raw -LiteralPath $planPath -Encoding UTF8 | ConvertFrom-Json
$root = (Resolve-Path -LiteralPath $info.root).Path
$runner = (Resolve-Path -LiteralPath $env:RUNNER_TEMP).Path.TrimEnd('\')
if (-not $root.StartsWith($runner+'\',[StringComparison]::OrdinalIgnoreCase) -or (Split-Path $root -Leaf) -notlike 'taskpulse-upgrade-*') { throw 'Unowned temporary root' }
if ($info.kind -notin @('nsis','msi') -or $planPath -ne (Join-Path $root 'plan.json')) { throw 'Windows owned plan required' }
$marker = Get-Content -Raw -LiteralPath (Join-Path $root 'ci-owned.json') | ConvertFrom-Json
if ($marker.commit -ne $info.commit) { throw 'Ownership marker mismatch' }
$repo = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$evidence = (Resolve-Path -LiteralPath $info.evidence).Path
if (-not $evidence.StartsWith($repo+'\dist\',[StringComparison]::OrdinalIgnoreCase)) { throw 'Evidence outside workspace dist' }
$name = 'tpup' + [Guid]::NewGuid().ToString('N').Substring(0,12)
# Password arrives through stdin; never an argument, file, env variable or log.
$password = [Console]::In.ReadToEnd().Trim() | ConvertTo-SecureString -AsPlainText -Force
$account = $null
$child = $null
$started = $false
$desktop = $null
$processesStopped = $false
$outcome = @{mode='dedicated-standard-user';created=$false;removed=$false;exit_code=$null}
$installerPolicy = $null
try {
    if ($info.kind -eq 'msi') {
        . (Join-Path $PSScriptRoot 'upgrade-managed-msi.ps1')
        $installerPolicy = Get-UpgradeInstallerPolicy
        Initialize-ManagedMsiFixture $info $root $evidence
        $outcome.mode = 'managed-msi-standard-user'
    }
    $account = New-LocalUser -Name $name -Password $password -AccountNeverExpires -Description 'Disposable TaskPulse upgrade acceptance'
    $outcome.created = $true
    $outcome.sid = $account.SID.Value
    Add-LocalGroupMember -SID 'S-1-5-32-545' -Member $account
    if (Get-LocalGroupMember -SID 'S-1-5-32-544' | Where-Object {$_.SID -eq $account.SID}) { throw 'Test account must not be an administrator' }
    foreach ($entry in @(@($root,'M'),@($repo,'RX'),@($evidence,'M'))) {
        & icacls.exe $entry[0] /grant ('*'+$account.SID.Value+':(OI)(CI)'+$entry[1]) | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'Could not grant scoped test-account access' }
    }
    # A new interactive logon needs access to this CI window station/desktop.
    # Grant only its SID, retain original DACLs, and restore them on exit.
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Security.Principal;
public sealed class TaskPulseDesktopAccess : IDisposable {
    [StructLayout(LayoutKind.Sequential)] struct Trustee { public IntPtr multiple; public int operation,form,type; public IntPtr name; }
    [StructLayout(LayoutKind.Sequential)] struct Access { public uint permissions; public int mode; public uint inheritance; public Trustee trustee; }
    [DllImport("user32.dll")] static extern IntPtr GetProcessWindowStation();
    [DllImport("user32.dll")] static extern IntPtr GetThreadDesktop(uint thread);
    [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
    [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr pointer);
    [DllImport("advapi32.dll")] static extern uint GetSecurityInfo(IntPtr handle,int kind,uint flags,out IntPtr owner,out IntPtr group,out IntPtr dacl,out IntPtr sacl,out IntPtr descriptor);
    [DllImport("advapi32.dll")] static extern uint SetSecurityInfo(IntPtr handle,int kind,uint flags,IntPtr owner,IntPtr group,IntPtr dacl,IntPtr sacl);
    [DllImport("advapi32.dll",CharSet=CharSet.Unicode)] static extern uint SetEntriesInAclW(uint count,ref Access access,IntPtr oldAcl,out IntPtr newAcl);
    IntPtr[] handles = new IntPtr[2], descriptors = new IntPtr[2], original = new IntPtr[2];
    public TaskPulseDesktopAccess(string sidText) {
        var sid=new SecurityIdentifier(sidText);var bytes=new byte[sid.BinaryLength];sid.GetBinaryForm(bytes,0);
        var pin=GCHandle.Alloc(bytes,GCHandleType.Pinned);
        handles[0]=GetProcessWindowStation();handles[1]=GetThreadDesktop(GetCurrentThreadId());
        try {
            for(int i=0;i<2;i++) {
                IntPtr owner,group,sacl,newAcl;
                uint error=GetSecurityInfo(handles[i],7,4,out owner,out group,out original[i],out sacl,out descriptors[i]);
                if(error!=0)throw new Win32Exception((int)error);
                if(original[i]==IntPtr.Zero)continue;
                var access=new Access {permissions=i==0?0x000F037Fu:0x000F01FFu,mode=1,trustee=new Trustee {form=0,type=1,name=pin.AddrOfPinnedObject()}};
                error=SetEntriesInAclW(1,ref access,original[i],out newAcl);
                if(error!=0)throw new Win32Exception((int)error);
                try {error=SetSecurityInfo(handles[i],7,4,IntPtr.Zero,IntPtr.Zero,newAcl,IntPtr.Zero);if(error!=0)throw new Win32Exception((int)error);}
                finally {LocalFree(newAcl);}
            }
        } catch {Dispose();throw;} finally {pin.Free();}
    }
    public void Dispose() {
        uint failure=0;
        for(int i=0;i<2;i++)if(descriptors[i]!=IntPtr.Zero) {
            uint error=SetSecurityInfo(handles[i],7,4,IntPtr.Zero,IntPtr.Zero,original[i],IntPtr.Zero);
            if(error!=0)failure=error;
            LocalFree(descriptors[i]);descriptors[i]=IntPtr.Zero;
        }
        if(failure!=0)throw new Win32Exception((int)failure);
    }
}
'@
    $desktop = [TaskPulseDesktopAccess]::new($account.SID.Value)
    $info | Add-Member -NotePropertyName standard_user_sid -NotePropertyValue $account.SID.Value -Force
    [IO.File]::WriteAllText($planPath, ($info | ConvertTo-Json -Depth 10), [Text.UTF8Encoding]::new($false))
    $child = [Diagnostics.Process]::new()
    $child.StartInfo.FileName = (Resolve-Path -LiteralPath $Python).Path
    $child.StartInfo.Arguments = '"'+(Join-Path $PSScriptRoot 'upgrade-smoke.py')+'" user-test --plan "'+$planPath+'"'
    $child.StartInfo.WorkingDirectory = $root
    $child.StartInfo.UseShellExecute = $false
    $child.StartInfo.RedirectStandardOutput = $true
    $child.StartInfo.RedirectStandardError = $true
    $child.StartInfo.LoadUserProfile = $true
    $child.StartInfo.UserName = $name
    $child.StartInfo.Domain = $env:COMPUTERNAME
    $child.StartInfo.Password = $password
    $child.StartInfo.EnvironmentVariables.Clear()
    foreach ($key in @('SystemRoot','SystemDrive','COMSPEC','GITHUB_ACTIONS','RUNNER_ENVIRONMENT','RUNNER_TEMP')) {
        $child.StartInfo.EnvironmentVariables[$key] = [Environment]::GetEnvironmentVariable($key)
    }
    $child.StartInfo.EnvironmentVariables['PATH'] = "$env:SystemRoot\System32;$env:SystemRoot"
    $child.StartInfo.EnvironmentVariables['PYTHONUTF8'] = '1'
    $started = $child.Start()
    if (-not $started) { throw 'Could not start standard-user acceptance' }
    $stdout = $child.StandardOutput.ReadToEndAsync()
    $stderr = $child.StandardError.ReadToEndAsync()
    if (-not $child.WaitForExit(600000)) { throw 'Standard-user acceptance timed out' }
    $child.WaitForExit()
    [IO.File]::WriteAllText((Join-Path $evidence 'standard-user.stdout.log'), $stdout.GetAwaiter().GetResult())
    [IO.File]::WriteAllText((Join-Path $evidence 'standard-user.stderr.log'), $stderr.GetAwaiter().GetResult())
    $outcome.exit_code = $child.ExitCode
    if ($child.ExitCode -ne 0) { throw 'Standard-user acceptance failed; see native evidence' }
} finally {
    try {
        if ($started -and -not $child.HasExited) { $child.Kill();[void]$child.WaitForExit(10000) }
        if ($account) {
            # The random SID was created by this invocation and has no unrelated
            # work. Include installer/WebView descendants even if Python crashed.
            for ($attempt=0;$attempt -lt 5;$attempt++) {
                $owned = @()
                foreach ($row in Get-CimInstance Win32_Process) {
                    $owner = Invoke-CimMethod -InputObject $row -MethodName GetOwnerSid -ErrorAction SilentlyContinue
                    if ($owner.Sid -eq $account.SID.Value) { $owned += $row }
                }
                if ($owned.Count -eq 0) { $processesStopped=$true;break }
                foreach ($row in $owned) {
                    $process = Get-Process -Id $row.ProcessId -ErrorAction SilentlyContinue
                    if (-not $process) { continue }
                    try {
                        $null = $process.Handle
                        $live = Get-CimInstance Win32_Process -Filter ("ProcessId="+$process.Id)
                        $owner = if ($live) { Invoke-CimMethod -InputObject $live -MethodName GetOwnerSid -ErrorAction SilentlyContinue }
                        if (-not $process.HasExited -and $owner.Sid -eq $account.SID.Value) {
                            $process.Kill()
                            if (-not $process.WaitForExit(10000)) { throw 'Test-account process did not stop' }
                        }
                    } finally { $process.Dispose() }
                }
            }
            if (-not $processesStopped) { throw 'Account cleanup refused while owned processes remain' }
        }
    } finally {
        try {
            if ($desktop) { $desktop.Dispose() }
        } finally {
            try {
                if ($child) { $child.Dispose() }
                if ($account -and $processesStopped) {
                    $found=Get-LocalUser -SID $account.SID -ErrorAction SilentlyContinue
                    if ($found -and $found.Name -ne $name) { throw 'Test-account ownership changed' }
                    if ($found) { Remove-LocalUser -SID $account.SID }
                    $outcome.removed = $true
                }
            } finally {
                $outcome.processes_stopped = $processesStopped
                $outcome | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $evidence 'standard-user.json') -Encoding UTF8
                $password.Dispose()
                if ($installerPolicy) {
                    $after = Get-UpgradeInstallerPolicy
                    @{before=($installerPolicy|ConvertFrom-Json);after=($after|ConvertFrom-Json);unchanged=($installerPolicy -eq $after)} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $evidence 'msi-policy.json') -Encoding UTF8
                    Get-WinEvent -FilterHashtable @{LogName='Application';ProviderName='MsiInstaller';StartTime=(Get-Date).AddMinutes(-15)} -ErrorAction SilentlyContinue | Select-Object TimeCreated,Id,Message | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $evidence 'msi-system-events.json') -Encoding UTF8
                    if ($installerPolicy -ne $after) { throw 'MSI/UAC policies changed during acceptance' }
                }
            }
        }
    }
}
