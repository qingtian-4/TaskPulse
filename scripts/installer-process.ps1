# Shared by CI installer acceptance and its non-installing native process tests.
# Keep the Process handle created by Start(): Start-Process -PassThru followed by
# WaitForExit can report a null ExitCode for a child which has already exited.
function Invoke-InstallerProcess {
    param(
        [Parameter(Mandatory=$true)][string]$FilePath,
        [string]$Arguments = '',
        [int]$TimeoutMilliseconds = 120000
    )
    if ($TimeoutMilliseconds -le 0) { throw 'A positive process timeout is required' }
    $resolvedExe = (Resolve-Path -LiteralPath $FilePath).Path
    $child = New-Object Diagnostics.Process
    $started = $false
    try {
        $child.StartInfo.FileName = $resolvedExe
        $child.StartInfo.Arguments = $Arguments
        $child.StartInfo.UseShellExecute = $false
        $child.StartInfo.CreateNoWindow = $true
        $child.StartInfo.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
        $child.StartInfo.RedirectStandardOutput = $true
        $child.StartInfo.RedirectStandardError = $true
        $started = $child.Start()
        if (-not $started) { throw 'Could not start acceptance child process' }
        $stdout = $child.StandardOutput.ReadToEndAsync()
        $stderr = $child.StandardError.ReadToEndAsync()
        if (-not $child.WaitForExit($TimeoutMilliseconds)) {
            throw [TimeoutException]::new("Acceptance child timed out: $resolvedExe")
        }
        $child.WaitForExit()
        [PSCustomObject]@{ExitCode=$child.ExitCode;StdOut=$stdout.GetAwaiter().GetResult();StdErr=$stderr.GetAwaiter().GetResult()}
    } finally {
        if ($started -and -not $child.HasExited) {
            $live = Get-Process -Id $child.Id -ErrorAction SilentlyContinue
            if ($live -and [IO.Path]::GetFullPath($live.Path) -eq $resolvedExe) {
                # This exact child and executable only; never stop by process name.
                $child.Kill()
                [void]$child.WaitForExit(5000)
            }
        }
        $child.Dispose()
    }
}
