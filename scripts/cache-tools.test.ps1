$ErrorActionPreference='Stop'
$fixture=Join-Path ([IO.Path]::GetTempPath()) ('taskpulse-cache-test-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path (Join-Path $fixture 'src-tauri\target\debug\incremental') -Force | Out-Null
[IO.File]::WriteAllText((Join-Path $fixture 'package.json'),'{}')
[IO.File]::WriteAllText((Join-Path $fixture 'src-tauri\Cargo.toml'),'# isolated fixture')
[IO.File]::WriteAllBytes((Join-Path $fixture 'src-tauri\target\debug\incremental\cached.bin'),(New-Object byte[] 1024))
New-Item -ItemType Directory -Path (Join-Path $fixture '.git') | Out-Null
[IO.File]::WriteAllText((Join-Path $fixture '.git\keep'),'history')
$preview=@(& (Join-Path $PSScriptRoot 'clean-build-cache.ps1') -Root $fixture -Caches debug-incremental)
if ($preview[0].Action -ne 'Dry-run' -or $preview[0].Bytes -ne 1024) { throw 'Dry-run did not report expected bytes.' }
if (-not (Test-Path -LiteralPath $preview[0].Path)) {throw 'Dry-run changed files.'}
$report=(& (Join-Path $PSScriptRoot 'disk-report.ps1') -Root $fixture -StateRoot (Join-Path $fixture 'absent-state') -Json) | ConvertFrom-Json
if ($report.TotalWorkspaceBytes -le 1024 -or $report.ScanErrors -ne 0) { throw 'Disk accounting failed.' }
$rejected=$false
try { & (Join-Path $PSScriptRoot 'clean-build-cache.ps1') -Root $fixture -Caches '..\..\.git' } catch { $rejected=$true }
if (-not $rejected) { throw 'Non-allowlisted cleanup was accepted.' }
$junction=Join-Path $fixture 'src-tauri\target\debug\build'
New-Item -ItemType Junction -Path $junction -Target (Join-Path $fixture '.git') | Out-Null
$rejected=$false
try { & (Join-Path $PSScriptRoot 'clean-build-cache.ps1') -Root $fixture -Caches debug-build -Delete -Confirm:$false } catch { $rejected=$true }
if (-not $rejected) { throw 'Junction cleanup was accepted.' }
if (-not $fixture.StartsWith([IO.Path]::GetTempPath(),[StringComparison]::OrdinalIgnoreCase)) {throw 'Fixture escaped temp.'}
& (Join-Path $PSScriptRoot 'clean-build-cache.ps1') -Root $fixture -Caches debug-incremental -Delete -Confirm:$false | Out-Null
if (Test-Path -LiteralPath $preview[0].Path) {throw 'Isolated deletion failed.'}
if (-not (Test-Path -LiteralPath (Join-Path $fixture '.git\keep'))) {throw 'Cleanup touched history.'}
Write-Output 'PASS: disk accounting, default dry-run, allowlist, isolated explicit deletion, history preservation. Fixture retained in temp.'
