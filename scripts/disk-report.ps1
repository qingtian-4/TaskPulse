param(
    [string]$Root,
    [string]$StateRoot = (Join-Path ([Environment]::GetFolderPath('UserProfile')) '.taskpulse'),
    [ValidateRange(1,100)][int]$Top = 15,
    [switch]$Json
)
. (Join-Path $PSScriptRoot 'cache-common.ps1')
if (-not $Root) { $Root = Split-Path -Parent $PSScriptRoot }
$workspace = Resolve-CacheWorkspace $Root
$groups = @{}
$large = @()
$script:SkippedLinks = 0; $script:ScanErrors = 0
$timer = [Diagnostics.Stopwatch]::StartNew()
Get-CacheFiles $workspace | ForEach-Object {
    $relative = $_.FullName.Substring($workspace.Length+1).Replace('\','/')
    $category = switch -Regex ($relative) {
        '^src-tauri/target/.*/bundle/.*\.(exe|msi|msix|zip)$' { 'installer-artifacts'; break }
        '^src-tauri/target/debug/' { 'rust-debug-cache'; break }
        '^src-tauri/target/(release|ci-release)/' { 'rust-optimized-cache'; break }
        '^src-tauri/target/codex-check/' { 'rust-isolated-verification'; break }
        '^src-tauri/target/' { 'other-rust-artifacts'; break }
        '^node_modules/' { 'node-modules'; break }
        '^(build|\.svelte-kit)/' { 'frontend-build'; break }
        '^\.git/' { 'git-history-do-not-clean'; break }
        default { 'source-and-other' }
    }
    if (-not $groups.ContainsKey($category)) { $groups[$category] = [pscustomobject]@{ Category=$category; Bytes=0L; Files=0L } }
    $groups[$category].Bytes += $_.Length; $groups[$category].Files++
    $large += [pscustomobject]@{ Path=$relative; Bytes=$_.Length }
    if ($large.Count -gt $Top*2) { $large = @($large | Sort-Object Bytes -Descending | Select-Object -First $Top) }
}
$workspaceLinks=$script:SkippedLinks; $workspaceErrors=$script:ScanErrors
$state = Measure-CacheTree ([IO.Path]::GetFullPath($StateRoot))
$report = [pscustomobject]@{
    Workspace=$workspace; ElapsedMs=$timer.ElapsedMilliseconds
    Categories=@($groups.Values | Sort-Object Bytes -Descending)
    TotalWorkspaceBytes=($groups.Values | Measure-Object Bytes -Sum).Sum
    TopFiles=@($large | Sort-Object Bytes -Descending | Select-Object -First $Top)
    SkippedLinks=$workspaceLinks; ScanErrors=$workspaceErrors
    UserState=$state; Note='Read-only; category totals do not overlap. User state is separate and must not be cleaned.'
}
if ($Json) { $report | ConvertTo-Json -Depth 6 } else {
    $report.Categories | Format-Table Category,@{n='GiB';e={[math]::Round($_.Bytes/1GB,3)}},Files -AutoSize
    $report.TopFiles | Format-Table Path,@{n='MiB';e={[math]::Round($_.Bytes/1MB,2)}} -AutoSize
    $report | Select-Object TotalWorkspaceBytes,ElapsedMs,SkippedLinks,ScanErrors,UserState,Note | Format-List
}
