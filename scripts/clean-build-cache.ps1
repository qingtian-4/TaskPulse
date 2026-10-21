[CmdletBinding(SupportsShouldProcess=$true,ConfirmImpact='High')]
param(
    [string]$Root,
    [ValidateSet('debug-incremental','debug-deps','debug-build','check-incremental','check-deps','frontend')]
    [string[]]$Caches = @('debug-incremental','debug-build','check-incremental'),
    [switch]$Delete
)
. (Join-Path $PSScriptRoot 'cache-common.ps1')
if (-not $Root) { $Root = Split-Path -Parent $PSScriptRoot }
$workspace = Resolve-CacheWorkspace $Root
$allowed = @{
    'debug-incremental'='src-tauri\target\debug\incremental'
    'debug-deps'='src-tauri\target\debug\deps'
    'debug-build'='src-tauri\target\debug\build'
    'check-incremental'='src-tauri\target\codex-check\debug\incremental'
    'check-deps'='src-tauri\target\codex-check\debug\deps'
    'frontend'='.svelte-kit'
}
foreach ($cache in ($Caches | Select-Object -Unique)) {
    $target = Assert-CacheTarget $workspace $allowed[$cache]
    $summary = Measure-CacheTree $target
    if ($Delete -and (Test-Path -LiteralPath $target)) {
        # Refuse concurrent compilers and any executable actually inside the
        # target. Never stop processes or weaken ownership to a basename match.
        $busy = Get-CimInstance Win32_Process | Where-Object {
            $_.Name -in @('cargo.exe','rustc.exe','rustup.exe') -or
            ($_.ExecutablePath -and [IO.Path]::GetFullPath($_.ExecutablePath).StartsWith($target+'\',[StringComparison]::OrdinalIgnoreCase))
        }
        if ($busy) { throw 'A compiler or target-owned process is running. Nothing was stopped; retry after it exits.' }
        $target = Assert-CacheTarget $workspace $allowed[$cache]
        if ($PSCmdlet.ShouldProcess($target,'Remove this allowlisted rebuildable cache only')) {
            Remove-Item -LiteralPath $target -Recurse -Force -ErrorAction Stop
            [pscustomobject]@{ Cache=$cache; Path=$target; Action='Removed'; Bytes=$summary.Bytes }
        }
    } else {
        [pscustomobject]@{ Cache=$cache; Path=$target; Action='Dry-run'; Bytes=$summary.Bytes; Files=$summary.Files }
    }
}
