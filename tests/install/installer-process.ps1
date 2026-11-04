# Shared by CI installer acceptance and its non-installing native process tests.
# Keep the Process handle created by Start(): Start-Process -PassThru followed by
# WaitForExit can report a null ExitCode for a child which has already exited.
function Initialize-InstallerPaths {
    if ('TaskPulse.InstallerPaths' -as [type]) { return }
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;
namespace TaskPulse {
  public static class InstallerPaths {
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern uint GetFinalPathNameByHandleW(SafeFileHandle file, StringBuilder path, uint size, uint flags);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern uint GetShortPathNameW(string path, StringBuilder shortPath, uint size);
    public static string Canonical(string path) {
      using(var file = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete)) {
        var buffer = new StringBuilder(32768);
        uint length = GetFinalPathNameByHandleW(file.SafeFileHandle, buffer, (uint)buffer.Capacity, 0);
        if(length == 0) throw new Win32Exception(Marshal.GetLastWin32Error());
        if(length >= buffer.Capacity) throw new IOException("Final path exceeds buffer");
        return buffer.ToString();
      }
    }
    public static string Short(string path) {
      var buffer = new StringBuilder(32768);
      uint length = GetShortPathNameW(path, buffer, (uint)buffer.Capacity);
      if(length == 0) throw new Win32Exception(Marshal.GetLastWin32Error());
      if(length >= buffer.Capacity) throw new IOException("Short path exceeds buffer");
      return buffer.ToString();
    }
  }
}
'@
}
function Test-InstallerSameExecutable([string]$First, [string]$Second) {
    if (-not $First -or -not $Second) { return $false }
    Initialize-InstallerPaths
    return [String]::Equals([TaskPulse.InstallerPaths]::Canonical($First),[TaskPulse.InstallerPaths]::Canonical($Second),[StringComparison]::OrdinalIgnoreCase)
}
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
            if ($live -and (Test-InstallerSameExecutable $live.Path $resolvedExe)) {
                # This exact child and executable only; never stop by process name.
                $child.Kill()
                [void]$child.WaitForExit(5000)
            }
        }
        $child.Dispose()
    }
}
