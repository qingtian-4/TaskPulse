$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '.\install\installer-process.ps1')
$commandExe = Join-Path $env:SystemRoot 'System32\cmd.exe'
for ($attempt=0;$attempt -lt 5;$attempt++) {
    $value = Invoke-InstallerProcess -FilePath $commandExe -Arguments '/d /c echo TaskPulse-process-ok' -TimeoutMilliseconds 5000
    if ($null -eq $value.ExitCode -or $value.ExitCode -ne 0 -or $value.StdOut.Trim() -ne 'TaskPulse-process-ok') { throw 'Fast successful exit/output was lost' }
}
$failed = Invoke-InstallerProcess -FilePath $commandExe -Arguments '/d /c exit 7' -TimeoutMilliseconds 5000
if ($failed.ExitCode -ne 7) { throw 'Nonzero exit code was lost' }
$powerShellExe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
Initialize-InstallerPaths
$shortExe = [TaskPulse.InstallerPaths]::Short($powerShellExe)
if (-not (Test-InstallerSameExecutable $powerShellExe $shortExe)) { throw 'Short and long names of the same executable differ' }
if (Test-InstallerSameExecutable $commandExe $powerShellExe) { throw 'Distinct executables must never share ownership' }
$pathFixture = [IO.Path]::GetTempFileName()
try {
    $finalName = [TaskPulse.InstallerPaths]::Canonical($pathFixture)
    $shortName = [TaskPulse.InstallerPaths]::Short($pathFixture)
    if (-not (Test-InstallerSameExecutable $finalName $shortName)) { throw 'Temp-profile short/long file aliases lost identity' }
    Write-Output "Temp-profile alias identity passed; spelling differs: $(-not [String]::Equals($finalName.Substring(4),$shortName,[StringComparison]::OrdinalIgnoreCase))"
} finally {
    $tempBoundary = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    $fixtureAbsolute = [IO.Path]::GetFullPath($pathFixture)
    if (-not $fixtureAbsolute.StartsWith($tempBoundary,[StringComparison]::OrdinalIgnoreCase)) { throw 'Refusing to remove a fixture outside the test temp directory' }
    Remove-Item -LiteralPath $fixtureAbsolute
}
$timedOut = $false
try { $null = Invoke-InstallerProcess -FilePath $powerShellExe -Arguments '-NoProfile -Command Start-Sleep -Seconds 10' -TimeoutMilliseconds 100 }
catch [TimeoutException] { $timedOut = $true }
if (-not $timedOut) { throw 'Timeout was not enforced' }
Write-Output 'Fast zero/nonzero exits, output capture and bounded timeout passed (no installers executed).'
Write-Output "Final-path identity passed: $powerShellExe / $shortExe"

& {
    # The identity probe must still work when PowerShell provider path
    # resolution or the optional CIM query is unavailable.
    $probe = Join-Path $PSScriptRoot '.\native\native-process-evidence.ps1'
    function Join-Path { throw 'Injected provider path-resolution outage' }
    function Get-Process { throw 'Injected process cmdlet discovery outage' }
    function Get-CimInstance { throw 'Injected CIM outage' }
    $identity = & $probe -ProcessId $PID -Exe $powerShellExe | ConvertFrom-Json
    if ($identity.pid -ne $PID -or $identity.owned_executable -ne $true -or $identity.user_matches_caller -ne $true -or $identity.elevated -isnot [bool] -or $identity.integrity_rid -le 0) {
        throw 'Required Windows process identity evidence was lost'
    }
    $childQueryFailed = $false
    try { $null = & $probe -ProcessId $PID -Exe $powerShellExe -ChildrenOnly -DebugPort 9222 }
    catch {
        if ($_.Exception.Message -ne 'Injected CIM outage') { throw }
        $childQueryFailed = $true
    }
    if (-not $childQueryFailed) { throw 'Child diagnostics did not reach the injected CIM query' }
}
