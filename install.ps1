<#
.SYNOPSIS
Download, verify and open the official TaskPulse Windows x64 installer.
.DESCRIPTION
Stable releases by default. Use -Prerelease to include public alpha releases,
or -Version for an explicit published tag. Drafts are never installed.
The normal interactive installer handles installation, upgrades and uninstall.
.EXAMPLE
.\install.ps1 -Prerelease
.EXAMPLE
.\install.ps1 -Version 1.0.0 -DownloadOnly -OutputDirectory .\downloads
#>
[CmdletBinding()]
param(
    [string]$Version,
    [switch]$Prerelease,
    [switch]$DownloadOnly,
    [string]$OutputDirectory = '.',
    [ValidateSet('en','zh-CN')][string]$Language = $(if ((Get-UICulture).Name -like 'zh*') { 'zh-CN' } else { 'en' })
)

function Get-TaskPulseRelease {
    param([string]$Version, [bool]$Prerelease)
    $api = 'https://api.github.com/repos/qingtian-4/TaskPulse/releases'
    $headers = @{ 'User-Agent' = 'TaskPulse-Installer'; Accept = 'application/vnd.github+json' }
    if ($Version) {
        $tag = if ($Version.StartsWith('v')) { $Version } else { "v$Version" }
        if ($tag -cnotmatch '^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$') { throw 'Invalid version / Invalid -Version.' }
        $release = Invoke-RestMethod -Uri "$api/tags/$tag" -Headers $headers -TimeoutSec 30
    } elseif ($Prerelease) {
        $releases = @(Invoke-RestMethod -Uri "${api}?per_page=100" -Headers $headers -TimeoutSec 30)
        $release = $releases | Where-Object { -not $_.draft -and $_.published_at } |
            Sort-Object { [DateTimeOffset]$_.published_at } -Descending | Select-Object -First 1
    } else {
        $release = Invoke-RestMethod -Uri "$api/latest" -Headers $headers -TimeoutSec 30
    }
    if (-not $release -or $release.draft -or -not $release.published_at) { throw 'No public release available. Draft releases cannot be installed.' }
    if (-not $Version -and -not $Prerelease -and $release.prerelease) { throw 'Prerelease requires -Prerelease or an explicit -Version.' }
    if ($Version -and $release.tag_name -cne $tag) { throw 'Release tag mismatch.' }
    return $release
}

function Get-TaskPulseAssetUrl {
    param($Release, [string]$Name)
    $assets = @($Release.assets | Where-Object { $_.name -ceq $Name })
    if ($assets.Count -ne 1) { throw "Missing or duplicate release asset: $Name" }
    $expected = 'https://github.com/qingtian-4/TaskPulse/releases/download/' + $Release.tag_name + '/' + $Name
    if ($assets[0].browser_download_url -cne $expected) { throw "Unexpected release asset URL: $Name" }
    return $expected
}

function Test-TaskPulseDownload {
    param([string]$Directory, [string]$Tag)
    if ($Tag -cnotmatch '^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$') { throw 'Invalid release tag.' }
    $version = $Tag.Substring(1)
    $name = "TaskPulse_${version}_windows_x64_setup.exe"
    $manifest = Get-Content -LiteralPath (Join-Path $Directory 'release-manifest.json') -Raw | ConvertFrom-Json
    if ($manifest.version -cne $version -or $manifest.tag -cne $Tag -or
        ($manifest.build_kind -and $manifest.build_kind -cne 'release') -or
        $manifest.platform -cne 'windows' -or $manifest.architecture -cne 'x64' -or
        $manifest.commit -cnotmatch '^[0-9a-f]{40}$' -or @($manifest.assets) -cnotcontains $name) {
        throw 'Release manifest does not match the requested Windows x64 version.'
    }
    $checksums = @(Get-Content -LiteralPath (Join-Path $Directory 'SHA256SUMS.txt') | Where-Object {
        $_ -cmatch ('^([0-9a-fA-F]{64})\s+\*?' + [regex]::Escape($name) + '$')
    })
    if ($checksums.Count -ne 1) { throw 'Missing, malformed or duplicate installer checksum.' }
    $expected = ($checksums[0] -split '\s+', 2)[0]
    $path = Join-Path $Directory $name
    $stream = [IO.File]::OpenRead($path)
    try {
        # Get-FileHash is a module-provided convenience command that is absent
        # in some Windows PowerShell hosts. .NET SHA256 works in both PS 5/7.
        $hasher = [Security.Cryptography.SHA256]::Create()
        try { $actual = [BitConverter]::ToString($hasher.ComputeHash($stream)).Replace('-', '') }
        finally { $hasher.Dispose() }
        if ($actual -ine $expected) { throw 'SHA-256 checksum mismatch. Nothing was installed.' }
        $stream.Position = 0
        if ($stream.ReadByte() -ne 0x4d -or $stream.ReadByte() -ne 0x5a) { throw 'Not a Windows executable.' }
    } finally { $stream.Dispose() }
    return $path
}

function Invoke-TaskPulseInstall {
    param([string]$Version, [bool]$Prerelease, [bool]$DownloadOnly, [string]$OutputDirectory, [string]$Language)
    $ErrorActionPreference = 'Stop'
    $zh = $Language -eq 'zh-CN'
    # PowerShell 5.1 source stays ASCII; localized messages use JSON escapes.
    $messages = if ($zh) { ConvertFrom-Json '{"download":"\u4e0b\u8f7d TaskPulse","verified":"SHA-256 \u6821\u9a8c\u901a\u8fc7","unsigned":"\u5b89\u88c5\u5305\u5c1a\u672a\u4ee3\u7801\u7b7e\u540d\uff0cWindows \u53ef\u80fd\u663e\u793a\u672a\u77e5\u53d1\u5e03\u8005\u3002","open":"\u6253\u5f00\u6807\u51c6\u5b89\u88c5\u5411\u5bfc","saved":"\u5df2\u4fdd\u5b58\u6821\u9a8c\u540e\u7684\u5b89\u88c5\u5305","failed":"\u5b89\u88c5\u672a\u5b8c\u6210\u3002\u8bf7\u68c0\u67e5\u7f51\u7edc\u8fde\u63a5\u548c GitHub Releases \u9875\u9762\u3002"}' }
        else { @{ download='Downloading TaskPulse'; verified='SHA-256 verified'; unsigned='The installer is not code signed. Windows may show an unknown publisher.'; open='Opening the standard setup wizard'; saved='Verified installer saved'; failed='Installation did not complete. Check your network and the GitHub Releases page.' } }
    $temp = $null
    try {
        $arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
        if ($env:OS -ne 'Windows_NT' -or $arch -ne 'AMD64' -or [Environment]::OSVersion.Version.Major -lt 10) { throw 'This installer supports Windows 10/11 x64 only.' }
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
        $release = Get-TaskPulseRelease $Version $Prerelease
        $tag = [string]$release.tag_name
        if ($tag -cnotmatch '^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$') { throw 'Invalid release tag.' }
        $name = 'TaskPulse_' + $tag.Substring(1) + '_windows_x64_setup.exe'
        $names = @($name, 'SHA256SUMS.txt', 'release-manifest.json')
        # Validate all asset locations before creating files or downloading.
        $urls = @($names | ForEach-Object { Get-TaskPulseAssetUrl $release $_ })
        $destination = $null
        $stagingRoot = [IO.Path]::GetTempPath()
        if ($DownloadOnly) {
            $outputProvider = $null
            $outputDrive = $null
            $stagingRoot = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputDirectory, [ref]$outputProvider, [ref]$outputDrive)
            if ($outputProvider.Name -ne 'FileSystem') { throw 'OutputDirectory must be a filesystem path.' }
            $destination = Join-Path $stagingRoot ('TaskPulse-' + $tag)
            if (Test-Path -LiteralPath $destination) { throw "Destination already exists: $destination" }
            [void][IO.Directory]::CreateDirectory($stagingRoot)
        }
        # Download-only staging stays on the output volume for atomic publication.
        $temp = Join-Path $stagingRoot ('taskpulse-install-' + [Guid]::NewGuid().ToString('N'))
        [void][IO.Directory]::CreateDirectory($temp)
        Write-Host "$($messages.download) $tag"
        for ($i=0; $i -lt $names.Count; $i++) {
            Invoke-WebRequest -UseBasicParsing -Uri $urls[$i] -OutFile (Join-Path $temp $names[$i]) -TimeoutSec 300
        }
        $installer = Test-TaskPulseDownload $temp $tag
        Write-Host $messages.verified
        Write-Host $messages.unsigned
        if ($DownloadOnly) {
            # Keep an owned version directory; never overwrite a previous download.
            # Unlike Move-Item, Directory.Move fails when a concurrent download
            # creates the destination; it cannot nest our staging directory there.
            [IO.Directory]::Move($temp, $destination)
            $temp = $null
            Write-Host "$($messages.saved): $destination"
            return
        }
        Write-Host $messages.open
        # Hold the real process handle so fast exits retain a valid ExitCode.
        # Keep normal installer permissions and UI; no silent mode or process killing.
        $child = New-Object Diagnostics.Process
        try {
            $child.StartInfo.FileName = $installer
            $child.StartInfo.UseShellExecute = $true
            if (-not $child.Start()) { throw 'Could not open the setup wizard.' }
            $child.WaitForExit()
            if ($child.ExitCode -ne 0) { throw "Setup exited with code $($child.ExitCode)." }
        } finally { $child.Dispose() }
    } catch {
        throw "$($messages.failed)`n$($_.Exception.Message)"
    } finally {
        if ($temp -and (Test-Path -LiteralPath $temp)) { Remove-Item -LiteralPath $temp -Recurse -Force }
    }
}

# Dot-sourcing exposes pure validation functions for offline tests.
if ($MyInvocation.InvocationName -ne '.') {
    Invoke-TaskPulseInstall -Version $Version -Prerelease $Prerelease.IsPresent -DownloadOnly $DownloadOnly.IsPresent -OutputDirectory $OutputDirectory -Language $Language
}
