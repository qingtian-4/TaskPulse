param(
    [Parameter(Mandatory=$true)][string]$Assets,
    [Parameter(Mandatory=$true)][string]$Version,
    [Parameter(Mandatory=$true)][string]$Gsudo,
    [Parameter(Mandatory=$true)][ValidateSet('nsis','msi')][string]$InstallerKind,
    [ValidateSet('x64','arm64')][string]$Architecture='x64',
    [switch]$VerifyRenamedExecutable
)
$ErrorActionPreference = 'Stop'
# This installs/uninstalls software. Never run it against a developer's real profile.
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'Run only on a disposable GitHub Actions Windows runner. Use the manual acceptance checklist on a clean VM.' }
if ($Version -notmatch '^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-(alpha|beta|rc)\.(0|[1-9]\d*))?$') { throw 'Invalid release version' }
$assetRoot = (Resolve-Path -LiteralPath $Assets).Path
$evidence = Join-Path (Get-Location).Path 'dist\installer-smoke'
New-Item -ItemType Directory -Path $evidence -Force | Out-Null
$stateRoot = Join-Path $env:USERPROFILE '.taskpulse'
if (Test-Path -LiteralPath $stateRoot) { throw 'Expected a fresh TaskPulse profile; refusing to overwrite existing data' }
$workspace = Join-Path $env:TEMP ('TaskPulse-installer-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $workspace | Out-Null
$marker = Join-Path $stateRoot 'installer-smoke-marker.txt'
New-Item -ItemType Directory -Path $stateRoot | Out-Null
Set-Content -LiteralPath $marker -Value 'preserve-user-data' -Encoding UTF8
$results = @()
$phase = 'prepare'
$failure = $null
$Gsudo = (Resolve-Path -LiteralPath $Gsudo).Path
. (Join-Path $PSScriptRoot '.\installer-process.ps1')

function Invoke-Installer([string]$File, [string]$Arguments) {
    $child = Invoke-InstallerProcess -FilePath $File -Arguments $Arguments
    if ($child.ExitCode -notin @(0,3010)) { throw "Installer exit $($child.ExitCode): $File" }
}

try {
    $manifest = Get-Content -Raw -Encoding UTF8 (Join-Path $assetRoot 'release-manifest.json') | ConvertFrom-Json
    if ($manifest.format) {
        if ($manifest.format -ne 'taskpulse-release-v2' -or $manifest.version -ne $Version -or $manifest.tag -ne "v$Version" -or $manifest.build_kind -ne 'release') { throw 'Unified release identity mismatch' }
        $rootCommit=$manifest.commit
        $manifest=$manifest.platforms.windows.$Architecture
        if ($manifest.commit -ne $rootCommit) { throw 'Platform source SHA mismatch' }
    }
    if ($manifest.version -ne $Version -or $manifest.commit -notmatch '^[a-f0-9]{40}$') { throw 'Release metadata mismatch' }
    if ($manifest.platform -ne 'windows' -or $manifest.architecture -ne $Architecture -or ($Architecture -eq 'arm64' -and $InstallerKind -ne 'nsis')) { throw 'Installer architecture/format mismatch' }
    if ($Architecture -eq 'x64') {
    $msiFile=Join-Path $assetRoot "TaskPulse_${Version}_windows_x64.msi"
    $windowsInstaller=New-Object -ComObject WindowsInstaller.Installer
    $database=$windowsInstaller.OpenDatabase($msiFile,0)
    $view=$database.OpenView("SELECT Value FROM Property WHERE Property='ProductVersion'")
    $view.Execute()
    $productVersion=$view.Fetch().StringData(1)
    $view.Close()
    if ($productVersion -ne $manifest.msi_product_version) { throw 'MSI numeric ProductVersion mismatch' }
    [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($view)
    [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($database)
    [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($windowsInstaller)
    }
    $expectedNames=@("TaskPulse_${Version}_windows_${Architecture}_setup.exe")
    if ($Architecture -eq 'x64') { $expectedNames+= "TaskPulse_${Version}_windows_x64.msi" }
    $seen=@{}
    foreach ($line in Get-Content -Encoding UTF8 (Join-Path $assetRoot 'SHA256SUMS.txt')) {
        if ($line -notmatch '^([a-f0-9]{64})  ([A-Za-z0-9_.-]+)$') { throw 'Invalid checksum path' }
        if ($Matches[2] -notin $expectedNames) { continue }
        if ($seen.ContainsKey($Matches[2])) { throw 'Duplicate installer checksum' }
        $seen[$Matches[2]]=$true
        if ((Get-FileHash -LiteralPath (Join-Path $assetRoot $Matches[2]) -Algorithm SHA256).Hash.ToLowerInvariant() -ne $Matches[1]) { throw 'Installer checksum mismatch' }
    }
    if ($seen.Count -ne $expectedNames.Count) { throw 'Missing installer checksum' }
    # One format per fresh runner: installer directory preferences survive uninstall.
    # Testing NSIS then MSI in one profile is a migration test, not a fresh install.
    foreach ($kind in @($InstallerKind)) {
        $phase = "$kind-install"
        Write-Host "[$kind] install"
        $installDir = Join-Path $workspace $kind
        New-Item -ItemType Directory -Path $installDir | Out-Null
        $exe = Join-Path $installDir 'TaskPulse.exe'
        $package = if ($kind -eq 'nsis') { Join-Path $assetRoot "TaskPulse_${Version}_windows_${Architecture}_setup.exe" } else { Join-Path $assetRoot "TaskPulse_${Version}_windows_x64.msi" }
        $arguments = if ($kind -eq 'nsis') { "/S /D=$installDir" } else { "/i `"$package`" /qn /norestart INSTALLDIR=`"$installDir`" /L*v `"$(Join-Path $evidence 'msi-install.log')`"" }
        $installer = if ($kind -eq 'nsis') { $package } else { Join-Path $env:SystemRoot 'System32\msiexec.exe' }
        Invoke-Installer $installer $arguments
        if (-not (Test-Path -LiteralPath $exe)) { throw "Missing installed executable: $exe" }
        $stream=[IO.File]::OpenRead($exe)
        $reader=[IO.BinaryReader]::new($stream)
        try {
            if ($reader.ReadUInt16() -ne 0x5a4d) { throw 'Invalid installed PE header' }
            $stream.Position=0x3c
            $peOffset=$reader.ReadUInt32()
            $stream.Position=$peOffset
            if ($reader.ReadUInt32() -ne 0x4550) { throw 'Invalid installed PE signature' }
            $machine=$reader.ReadUInt16()
            if (($Architecture -eq 'arm64' -and $machine -ne 0xaa64) -or ($Architecture -eq 'x64' -and $machine -ne 0x8664)) { throw 'Installed executable architecture mismatch' }
        } finally { $reader.Dispose(); $stream.Dispose() }
        foreach ($resource in @('agent-tray-hook.ps1','install-hooks.ps1')) {
            if (-not (Test-Path -LiteralPath (Join-Path $installDir "scripts\hooks\$resource"))) { throw "Missing packaged Hook resource: $resource" }
        }
        foreach ($resource in @('LICENSE','licenses\THIRD_PARTY_NOTICES.txt','licenses\DEPENDENCIES.json')) {
            if (-not (Test-Path -LiteralPath (Join-Path $installDir $resource))) { throw "Missing packaged license material: $resource" }
        }
        $phase = "$kind-version"
        $versionOutput = Join-Path $evidence "$kind-version.txt"
        $versionProcess = Invoke-InstallerProcess -FilePath $exe -Arguments '--version' -TimeoutMilliseconds 15000
        $versionProcess.StdOut | Set-Content -LiteralPath $versionOutput -Encoding UTF8
        $versionProcess | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $evidence "$kind-version-process.json") -Encoding UTF8
        if ($versionProcess.ExitCode -ne 0 -or $versionProcess.StdOut.Trim() -ne "TaskPulse $Version") { throw "Installed version check failed (exit $($versionProcess.ExitCode)): $($versionProcess.StdOut.Trim())" }
        $legacy = Join-Path $installDir 'agent-tray.exe'
        $bridgeHash = $null
        if ($VerifyRenamedExecutable) {
            $identityProcess = Invoke-InstallerProcess -FilePath $exe -Arguments '--profile-info' -TimeoutMilliseconds 15000
            if ($identityProcess.ExitCode -ne 0) { throw 'Installed build identity probe failed' }
            $identity = $identityProcess.StdOut | ConvertFrom-Json
            if ($identity.commit -ne $manifest.commit -or $identity.modified -ne $false -or $identity.channel -ne 'release' -or $identity.version -ne $Version) { throw 'Installed build identity mismatch' }
            $legacyLaunch = Invoke-InstallerProcess -FilePath $legacy -Arguments '--show' -TimeoutMilliseconds 15000
            if ($legacyLaunch.ExitCode -ne 2) { throw 'The old path must reject GUI launches' }
            $bridgeHash = (Get-FileHash -LiteralPath $legacy -Algorithm SHA256).Hash.ToLowerInvariant()
        }
        $phase = "$kind-native"
        # WebView2 >=150 ignores environment debug switches in elevated hosts.
        # Install with normal installer permissions, but drive the app as a user.
        $appSmoke = Join-Path $PSScriptRoot '.\installer-app-smoke.ps1'
        & $Gsudo --integrity Medium --direct powershell.exe -NoProfile -ExecutionPolicy Bypass -File $appSmoke -Exe $exe -Evidence $evidence -Kind $kind
        if ($LASTEXITCODE -ne 0) { throw "Medium-integrity native acceptance failed for $kind" }
        $native = Get-Content -Raw -Encoding UTF8 (Join-Path $evidence "$kind-native.json") | ConvertFrom-Json
        if ($native.phase -ne 'passed' -or $native.integrity -ne 'medium') { throw 'Missing native acceptance evidence' }
        if ($native.native_ipc -is [Array] -or $native.native_ipc.embedded -ne $true -or $native.native_ipc.onboarding -ne $false -or $native.native_ipc.tasks -ne 0 -or $native.native_ipc.prompts -ne 0) { throw 'Native acceptance evidence must be a single validated IPC result' }
        $phase = "$kind-reinstall"
        Write-Host "[$kind] reinstall"
        Invoke-Installer $installer $arguments
        if (-not (Test-Path -LiteralPath $marker)) { throw 'Reinstall removed user data' }
        $signature = (Get-AuthenticodeSignature -FilePath $package).Status.ToString()
        $applicationHash=(Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
        $packageHash=(Get-FileHash -LiteralPath $package -Algorithm SHA256).Hash.ToLowerInvariant()
        $phase = "$kind-uninstall"
        Write-Host "[$kind] uninstall"
        if ($kind -eq 'nsis') {
            $uninstaller = (Resolve-Path -LiteralPath (Join-Path $installDir 'uninstall.exe')).Path
            if (-not $uninstaller.StartsWith($workspace+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) { throw 'Uninstaller escaped smoke workspace' }
            Invoke-Installer $uninstaller '/S'
        } else {
            Invoke-Installer $installer "/x `"$package`" /qn /norestart /L*v `"$(Join-Path $evidence 'msi-uninstall.log')`""
        }
        for($n=0;$n -lt 50 -and (Test-Path -LiteralPath $exe);$n++){Start-Sleep -Milliseconds 200}
        if (Test-Path -LiteralPath $exe) { throw 'Uninstall left the application executable installed' }
        if ($VerifyRenamedExecutable -and (Test-Path -LiteralPath $legacy)) { throw 'Uninstall left the legacy bridge installed' }
        if (-not (Test-Path -LiteralPath $marker)) { throw 'Uninstall removed user state without permission' }
        $results += @{kind=$kind;version=$Version;source_commit=$manifest.commit;os=[Environment]::OSVersion.VersionString;architecture=$Architecture;installation_method=$kind;script_sha256=(Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant();package_sha256=$packageHash;application_sha256=$applicationHash;legacy_bridge_sha256=$bridgeHash;signature=$signature;native_ipc=$native.native_ipc;restart=$native.restart;reinstall=$true;uninstall=$true;cross_version_upgrade='not_verified';format_migration='not_verified';focus='not_verified'}
    }
    $phase = 'passed'
} catch {
    $failure = $_.Exception.Message
    throw
} finally {
    @{version=$Version;installer=$InstallerKind;phase=$phase;error=$failure;results=@($results)} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $evidence 'result.json') -Encoding UTF8
}
