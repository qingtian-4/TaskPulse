$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'installer-process.ps1')
$commandExe = Join-Path $env:SystemRoot 'System32\cmd.exe'
for ($attempt=0;$attempt -lt 5;$attempt++) {
    $value = Invoke-InstallerProcess -FilePath $commandExe -Arguments '/d /c echo TaskPulse-process-ok' -TimeoutMilliseconds 5000
    if ($null -eq $value.ExitCode -or $value.ExitCode -ne 0 -or $value.StdOut.Trim() -ne 'TaskPulse-process-ok') { throw 'Fast successful exit/output was lost' }
}
$failed = Invoke-InstallerProcess -FilePath $commandExe -Arguments '/d /c exit 7' -TimeoutMilliseconds 5000
if ($failed.ExitCode -ne 7) { throw 'Nonzero exit code was lost' }
$powerShellExe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$timedOut = $false
try { $null = Invoke-InstallerProcess -FilePath $powerShellExe -Arguments '-NoProfile -Command Start-Sleep -Seconds 10' -TimeoutMilliseconds 100 }
catch [TimeoutException] { $timedOut = $true }
if (-not $timedOut) { throw 'Timeout was not enforced' }
Write-Output 'Fast zero/nonzero exits, output capture and bounded timeout passed (no installers executed).'
