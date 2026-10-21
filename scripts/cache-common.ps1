Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Resolve-CacheWorkspace([string]$Root) {
    $resolved = [IO.Path]::GetFullPath((Resolve-Path -LiteralPath $Root).Path).TrimEnd('\','/')
    if (-not (Test-Path -LiteralPath (Join-Path $resolved 'package.json') -PathType Leaf) -or
        -not (Test-Path -LiteralPath (Join-Path $resolved 'src-tauri\Cargo.toml') -PathType Leaf)) {
        throw 'Expected a TaskPulse workspace with package.json and src-tauri/Cargo.toml.'
    }
    $ancestor = Get-Item -LiteralPath $resolved -Force
    while ($null -ne $ancestor) {
        if ($ancestor.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Workspace ancestors must not be junctions/symlinks.' }
        $ancestor = $ancestor.Parent
    }
    return $resolved
}

function Get-CacheFiles([string]$Root) {
    if (-not (Test-Path -LiteralPath $Root)) { return }
    $stack = New-Object 'System.Collections.Generic.Stack[string]'
    $stack.Push([IO.Path]::GetFullPath($Root))
    while ($stack.Count -gt 0) {
        $path = $stack.Pop()
        try {
            $item = Get-Item -LiteralPath $path -Force
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { $script:SkippedLinks++; continue }
            if (-not $item.PSIsContainer) { $item; continue }
            Get-ChildItem -LiteralPath $path -Force -ErrorAction Stop | ForEach-Object {
                if ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) { $script:SkippedLinks++ }
                elseif ($_.PSIsContainer) { $stack.Push($_.FullName) }
                else { $_ }
            }
        } catch { $script:ScanErrors++ }
    }
}

function Measure-CacheTree([string]$Root) {
    $script:SkippedLinks = 0; $script:ScanErrors = 0
    $bytes = 0L; $count = 0L
    Get-CacheFiles $Root | ForEach-Object { $bytes += $_.Length; $count++ }
    [pscustomobject]@{ Path=$Root; Bytes=$bytes; Files=$count; SkippedLinks=$script:SkippedLinks; Errors=$script:ScanErrors }
}

function Assert-CacheTarget([string]$Root,[string]$Relative) {
    $target = [IO.Path]::GetFullPath((Join-Path $Root $Relative))
    if (-not $target.StartsWith($Root + [IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) { throw 'Target escapes workspace.' }
    if (Test-Path -LiteralPath $target) {
        $ancestor = Get-Item -LiteralPath $target -Force
        while ($ancestor.FullName -ne $Root) {
            if ($ancestor.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Cache target contains a junction/symlink.' }
            $ancestor = $ancestor.Parent
            if ($null -eq $ancestor) { throw 'Target ancestry escapes workspace.' }
        }
        $summary = Measure-CacheTree $target
        if ($summary.SkippedLinks -gt 0 -or $summary.Errors -gt 0) { throw 'Cache tree contains links or inaccessible paths; refusing deletion.' }
    }
    return $target
}
