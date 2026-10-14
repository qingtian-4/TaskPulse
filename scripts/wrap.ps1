# Compatibility entry: wrap.ps1 <name> <native executable> [arguments...]
# Supervises process boundaries, not internal Agent turns or approvals.
param(
    [Parameter(Mandatory=$true, Position=0)][string]$AgentName,
    [Parameter(Mandatory=$true, Position=1, ValueFromRemainingArguments=$true)][AllowEmptyString()][string[]]$Command
)
$ErrorActionPreference = 'Stop'
$jobDirectory = if ($env:TASKPULSE_STATUS_DIR) { $env:TASKPULSE_STATUS_DIR } else { Join-Path $env:USERPROFILE '.agent-monitor' }
$null = New-Item -ItemType Directory -Path $jobDirectory -Force
$jobId = [Guid]::NewGuid().ToString()
$jobStatus = Join-Path $jobDirectory "$jobId.status"
$jobStart = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
$jobActivity = $jobStart
$jobProcess = $null
$jobStarted = $false
$jobOutputCopy = $null
$jobErrorCopy = $null
$jobInputCopy = $null
$jobMessage = $Command[0]

# Windows CRT argv quoting, including empty strings, quotes and trailing slashes.
function Quote-JobArgument([string]$value) {
    $result = '"'
    $slashes = 0
    foreach ($character in $value.ToCharArray()) {
        if ($character -eq '\') { $slashes++; continue }
        if ($character -eq '"') { $result += ('\' * (2 * $slashes + 1)) + '"' }
        else { $result += ('\' * $slashes) + $character }
        $slashes = 0
    }
    return $result + ('\' * (2 * $slashes)) + '"'
}
function Write-JobStatus([string]$status, $code=$null, $finished=$null) {
    $value = @{
        v=2; name=$AgentName; run_id=$jobId; turn_key=$jobId; status=$status
        cwd=(Get-Location).Path; started_at=$jobStart; finished_at=$finished
        execution_activity_at=$jobActivity
        heartbeat_at=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        pid=$PID; exit_code=$code; confidence='exact'; message=$jobMessage
        activity=$(if ($status -eq 'running') { $Command[0] } else { $null })
    }
    $temporary = "$jobStatus.tmp"
    [System.IO.File]::WriteAllText($temporary, ($value | ConvertTo-Json -Compress), (New-Object System.Text.UTF8Encoding($false)))
    Move-Item -LiteralPath $temporary -Destination $jobStatus -Force
}
$jobExit = 1
try {
    $jobExecutable = (Get-Command $Command[0] -CommandType Application -ErrorAction Stop).Source
    if ([IO.Path]::GetExtension($jobExecutable) -in @('.cmd','.bat')) {
        throw 'Batch files require an explicit cmd.exe command; shell syntax is not reinterpreted.'
    }
    $jobProcess = New-Object System.Diagnostics.Process
    $jobProcess.StartInfo.FileName = $jobExecutable
    $jobProcess.StartInfo.WorkingDirectory = (Get-Location).ProviderPath
    if ($Command.Length -gt 1) {
        $jobProcess.StartInfo.Arguments = (($Command[1..($Command.Length - 1)] | ForEach-Object { Quote-JobArgument $_ }) -join ' ')
    }
    $jobProcess.StartInfo.UseShellExecute = $false
    $jobProcess.StartInfo.CreateNoWindow = $true
    # Keep terminal streams inherited. A redirected PowerShell host cannot
    # reliably inherit its pipes through Process.Start, so relay both pipes
    # concurrently as raw bytes (never scan prompts or drain sequentially).
    $jobProcess.StartInfo.RedirectStandardOutput = [Console]::IsOutputRedirected
    $jobProcess.StartInfo.RedirectStandardError = [Console]::IsErrorRedirected
    $jobProcess.StartInfo.RedirectStandardInput = [Console]::IsInputRedirected
    if ($jobProcess.StartInfo.RedirectStandardInput -and -not ('TaskPulseInputRelay' -as [type])) {
        Add-Type -TypeDefinition @'
using System; using System.IO; using System.Threading.Tasks;
public static class TaskPulseInputRelay {
    public static async Task Copy(Stream source, Stream target) {
        try { await source.CopyToAsync(target).ConfigureAwait(false); }
        catch (IOException) { } catch (ObjectDisposedException) { }
        finally { target.Dispose(); }
    }
}
'@
    }
    Write-JobStatus 'running'
    $null = $jobProcess.Start()
    $jobStarted = $true
    if ($jobProcess.StartInfo.RedirectStandardInput) {
        $jobInputCopy = [TaskPulseInputRelay]::Copy([Console]::OpenStandardInput(), $jobProcess.StandardInput.BaseStream)
    }
    if ($jobProcess.StartInfo.RedirectStandardOutput) {
        $jobOutputCopy = $jobProcess.StandardOutput.BaseStream.CopyToAsync([Console]::OpenStandardOutput())
    }
    if ($jobProcess.StartInfo.RedirectStandardError) {
        $jobErrorCopy = $jobProcess.StandardError.BaseStream.CopyToAsync([Console]::OpenStandardError())
    }
    while (-not $jobProcess.WaitForExit(1000)) { Write-JobStatus 'running' }
    if ($jobOutputCopy) { $null = $jobOutputCopy.GetAwaiter().GetResult() }
    if ($jobErrorCopy) { $null = $jobErrorCopy.GetAwaiter().GetResult() }
    $jobExit = $jobProcess.ExitCode
    $jobActivity = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    $jobResult = if ($jobExit -eq 0) { 'succeeded' } else { 'failed' }
    Write-JobStatus $jobResult $jobExit $jobActivity
} catch {
    $jobExit = 127
    $jobMessage = "运行监督器错误：$($_.Exception.GetType().Name)"
    $jobActivity = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    Write-JobStatus 'failed' $jobExit $jobActivity
    Write-Error $_ -ErrorAction Continue
} finally {
    if ($jobStarted -and -not $jobProcess.HasExited) {
        $jobProcess.Kill()
        $jobProcess.WaitForExit()
        $jobExit = 130
        $jobActivity = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        Write-JobStatus 'failed' $jobExit $jobActivity
    }
    if ($jobProcess) { $jobProcess.Dispose() }
}
exit $jobExit
