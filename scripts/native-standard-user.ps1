param([Parameter(Mandatory=$true)][string]$Bun,[Parameter(Mandatory=$true)][string]$Exe,[switch]$ChildMode,[string]$ExpectedSid,[string]$Evidence)
$ErrorActionPreference='Stop'
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') { throw 'Disposable hosted CI only' }
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$repo=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
. (Join-Path $PSScriptRoot 'installer-process.ps1')
if ($ChildMode) {
    $identity=[Security.Principal.WindowsIdentity]::GetCurrent()
    if (-not $ExpectedSid -or $identity.User.Value -ne $ExpectedSid -or ([Security.Principal.WindowsPrincipal]$identity).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Actual standard account required' }
    foreach($entry in @(@('USERPROFILE','UserProfile'),@('APPDATA','ApplicationData'),@('LOCALAPPDATA','LocalApplicationData'))) {
        $value=[Environment]::GetFolderPath([Environment+SpecialFolder]($entry[1]),[Environment+SpecialFolderOption]::Create)
        if(-not $value){throw 'Standard-account known folder unavailable'}
        [Environment]::SetEnvironmentVariable($entry[0],$value,'Process')
    }
    $env:TEMP=$env:TMP=Join-Path $env:LOCALAPPDATA 'Temp'
    [void][IO.Directory]::CreateDirectory($env:TEMP)
    $before=@(Get-ChildItem -LiteralPath (Join-Path $repo 'src-tauri\target\ui-smoke\evidence') -Directory -Filter 'development-native-*' -ErrorAction SilentlyContinue | ForEach-Object {$_.FullName})
    Set-Location $repo
    & $Bun (Join-Path $PSScriptRoot 'native-development-smoke.ts') --exe $Exe
    $code=$LASTEXITCODE
    $after=@(Get-ChildItem -LiteralPath (Join-Path $repo 'src-tauri\target\ui-smoke\evidence') -Directory -Filter 'development-native-*' | Where-Object {$_.FullName -notin $before})
    if($after.Count -ne 1){throw 'Expected one native standard-account result'}
    $resultPath=Join-Path $after[0].FullName 'result.json'
    if($code -ne 0 -or -not (Test-Path -LiteralPath $resultPath)){throw 'Native standard-account verification failed; see startup evidence'}
    $result=Get-Content -LiteralPath $resultPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if(-not $result.passed -or $result.startup.process.user_matches_caller -ne $true -or $result.startup.process.elevated -ne $false -or $result.startup.process.integrity_rid -ne 8192){throw 'Native app must run as the standard account at medium integrity with TokenElevation=0'}
    @{passed=$true;mode='dedicated-standard-user';sid=$identity.User.Value;native_result=$resultPath;app_elevated=$result.startup.process.elevated;integrity_rid=$result.startup.process.integrity_rid;commit=$result.commit} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Evidence 'native-user.json') -Encoding UTF8
    exit 0
}
if($env:TAURI_SIGNING_PRIVATE_KEY -or $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD){throw 'Signing material is forbidden in native PR verification'}
if(-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)){throw 'CI account setup requires its administrator'}
if((Resolve-Path -LiteralPath $env:GITHUB_WORKSPACE).Path -ne $repo){throw 'Unexpected workspace'}
$Exe=(Resolve-Path -LiteralPath $Exe).Path
$Bun=(Resolve-Path -LiteralPath $Bun).Path
if(-not $Exe.StartsWith($repo+'\src-tauri\target\',[StringComparison]::OrdinalIgnoreCase)){throw 'Native executable must be workspace-owned'}
$sha=(& git -C $repo rev-parse HEAD).Trim()
if($LASTEXITCODE -ne 0 -or $sha -notmatch '^[a-f0-9]{40}$'){throw 'Exact committed source required'}
$root=Join-Path (Resolve-Path -LiteralPath $env:RUNNER_TEMP).Path ('taskpulse-native-'+[Guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $root)
$runtime=Join-Path $root 'bun.exe';Copy-Item -LiteralPath $Bun -Destination $runtime
$bunHash=(Get-FileHash -LiteralPath $Bun -Algorithm SHA256).Hash
$exeHash=(Get-FileHash -LiteralPath $Exe -Algorithm SHA256).Hash
if((Get-FileHash -LiteralPath $runtime -Algorithm SHA256).Hash -ne $bunHash){throw 'Runtime copy mismatch'}
$Evidence=Join-Path $repo ('src-tauri\target\ui-smoke\evidence\standard-user-'+$sha+'-'+[Guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $Evidence)
$name='tpnv'+[Guid]::NewGuid().ToString('N').Substring(0,12)
$bytes=New-Object byte[] 48;$rng=[Security.Cryptography.RandomNumberGenerator]::Create();$rng.GetBytes($bytes);$rng.Dispose()
$password=([Convert]::ToBase64String($bytes)+'aA1!') | ConvertTo-SecureString -AsPlainText -Force
[Array]::Clear($bytes,0,$bytes.Length)
$account=$null;$desktop=$null;$child=$null;$started=$false;$stopped=$false;$acls=@()
$outcome=@{mode='dedicated-standard-user';commit=$sha;created=$false;removed=$false;processes_stopped=$false;exit_code=$null}
try {
    $account=New-LocalUser -Name $name -Password $password -AccountNeverExpires -Description 'Disposable TaskPulse native acceptance'
    $outcome.created=$true;$outcome.sid=$account.SID.Value
    Add-LocalGroupMember -SID 'S-1-5-32-545' -Member $account
    if(Get-LocalGroupMember -SID 'S-1-5-32-544' | Where-Object {$_.SID -eq $account.SID}){throw 'Test account must not be an administrator'}
    $access=@(@($repo,'RX'),@((Join-Path $repo 'src-tauri\target'),'M'),@($root,'M'))
    foreach($entry in $access){$acls+=@{path=$entry[0];acl=(Get-Acl -LiteralPath $entry[0])}}
    foreach($entry in $access) {
        & icacls.exe $entry[0] /grant ('*'+$account.SID.Value+':(OI)(CI)'+$entry[1]) | Out-Null
        if($LASTEXITCODE -ne 0){throw 'Could not grant scoped native fixture access'}
    }
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
    $desktop=[TaskPulseDesktopAccess]::new($account.SID.Value)
    $child=[Diagnostics.Process]::new();$child.StartInfo.FileName=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $child.StartInfo.Arguments='-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "'+$PSCommandPath+'" -ChildMode -Bun "'+$runtime+'" -Exe "'+$Exe+'" -ExpectedSid "'+$account.SID.Value+'" -Evidence "'+$Evidence+'"'
    $child.StartInfo.WorkingDirectory=$repo;$child.StartInfo.UseShellExecute=$false;$child.StartInfo.LoadUserProfile=$true
    $child.StartInfo.UserName=$name;$child.StartInfo.Domain=$env:COMPUTERNAME;$child.StartInfo.Password=$password
    $child.StartInfo.RedirectStandardOutput=$true;$child.StartInfo.RedirectStandardError=$true
    $child.StartInfo.EnvironmentVariables.Clear()
    foreach($key in @('SystemRoot','SystemDrive','COMSPEC','GITHUB_ACTIONS','RUNNER_ENVIRONMENT','RUNNER_TEMP')){$child.StartInfo.EnvironmentVariables[$key]=[Environment]::GetEnvironmentVariable($key)}
    $child.StartInfo.EnvironmentVariables['PATH']="$env:SystemRoot\System32;$env:SystemRoot;$env:SystemRoot\System32\WindowsPowerShell\v1.0"
    $started=$child.Start();if(-not $started){throw 'Could not start standard-account native acceptance'}
    $stdout=$child.StandardOutput.ReadToEndAsync();$stderr=$child.StandardError.ReadToEndAsync()
    if(-not $child.WaitForExit(180000)){throw 'Native standard-account acceptance timed out'}
    $child.WaitForExit();$outcome.exit_code=$child.ExitCode
    [IO.File]::WriteAllText((Join-Path $Evidence 'stdout.log'),$stdout.GetAwaiter().GetResult())
    [IO.File]::WriteAllText((Join-Path $Evidence 'stderr.log'),$stderr.GetAwaiter().GetResult())
    if($child.ExitCode -ne 0){throw 'Native standard-account acceptance failed; see retained evidence'}
    if((Get-FileHash -LiteralPath $Exe -Algorithm SHA256).Hash -ne $exeHash){throw 'Original native executable changed'}
} finally {
    try {
        if($started -and -not $child.HasExited){$child.Kill();[void]$child.WaitForExit(10000)}
        if($account){
            for($attempt=0;$attempt -lt 5;$attempt++){
                $owned=@(Get-CimInstance Win32_Process | Where-Object {(Invoke-CimMethod -InputObject $_ -MethodName GetOwnerSid -ErrorAction SilentlyContinue).Sid -eq $account.SID.Value})
                if($owned.Count -eq 0){$stopped=$true;break}
                foreach($row in $owned){
                    $process=Get-Process -Id $row.ProcessId -ErrorAction SilentlyContinue;if(-not $process){continue}
                    try {
                        $null=$process.Handle;$live=Get-CimInstance Win32_Process -Filter ('ProcessId='+$process.Id)
                        $owner=if($live){Invoke-CimMethod -InputObject $live -MethodName GetOwnerSid -ErrorAction SilentlyContinue}
                        if(-not $process.HasExited -and $owner.Sid -eq $account.SID.Value){$process.Kill();if(-not $process.WaitForExit(10000)){throw 'Owned native process did not stop'}}
                    } finally {$process.Dispose()}
                }
            }
            if(-not $stopped){throw 'Account cleanup refused while owned processes remain'}
        }
    } finally {
        if(-not $account -or $stopped){
            try {if($desktop){$desktop.Dispose()}} finally {
                for($i=$acls.Count-1;$i -ge 0;$i--){Set-Acl -LiteralPath $acls[$i].path -AclObject $acls[$i].acl}
                if($child){$child.Dispose()}
                if($account){$found=Get-LocalUser -SID $account.SID -ErrorAction SilentlyContinue;if($found -and $found.Name -ne $name){throw 'Account ownership changed'};if($found){Remove-LocalUser -SID $account.SID};$outcome.removed=$true}
            }
        }
        $outcome.processes_stopped=$stopped
        $outcome | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Evidence 'standard-user.json') -Encoding UTF8
        $password.Dispose()
    }
}
