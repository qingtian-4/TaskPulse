param(
    [Parameter(Mandatory=$true)][string]$Assets,
    [Parameter(Mandatory=$true)][string]$Version
)
$ErrorActionPreference='Stop'
if($MyInvocation.InvocationName -ne '.' -and $env:GITHUB_ACTIONS -ne 'true'){throw 'Only disposable Windows CI runners may run package acceptance'}
function Invoke-TaskPulseBootstrapProbe {
    param([string]$Assets,[string]$Version,[string]$Evidence)
    . (Join-Path $PSScriptRoot '..\install.ps1') -Version $Version
    $assetRoot=(Resolve-Path -LiteralPath $Assets).Path
    $evidence=$Evidence
    [void][IO.Directory]::CreateDirectory($evidence)
    $temp=Join-Path ([IO.Path]::GetTempPath()) ('taskpulse-bootstrap-'+[Guid]::NewGuid().ToString('N'))
    [void][IO.Directory]::CreateDirectory($temp)
    $passed=$false
    function Get-TaskPulseProbeHash([string]$Path) {
        $stream=[IO.File]::OpenRead($Path)
        try {
            $hasher=[Security.Cryptography.SHA256]::Create()
            try { return [BitConverter]::ToString($hasher.ComputeHash($stream)).Replace('-','').ToLowerInvariant() }
            finally { $hasher.Dispose() }
        } finally { $stream.Dispose() }
    }
    try {
        $manifestPath=Join-Path $assetRoot 'release-manifest.json'
        $preview=Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
        if($preview.version -cne $Version -or $preview.tag -ne $null -or $preview.build_kind -cne 'pr-preview' -or $preview.commit -cnotmatch '^[a-f0-9]{40}$'){throw 'Expected exact PR preview metadata'}
        $tag='v'+$Version
        $name="TaskPulse_${Version}_windows_x64_setup.exe"
        # Rejection is part of the public bootstrap contract, even with a release tag supplied.
        $rejected=$false
        try { Test-TaskPulseDownload $assetRoot $tag | Out-Null } catch { $rejected=$true }
        if(-not $rejected){throw 'Production validator accepted a PR preview as a public release'}
        # Only this test response is normalized. The downloaded real package and its
        # hash are unchanged. The uploaded preview manifest is never edited.
        $response=Join-Path $temp 'response'
        [void][IO.Directory]::CreateDirectory($response)
        foreach($file in @($name,'SHA256SUMS.txt')) {Copy-Item -LiteralPath (Join-Path $assetRoot $file) -Destination $response}
        $preview.tag=$tag
        $preview.build_kind='release'
        $preview | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $response 'release-manifest.json') -Encoding UTF8
        $script:release=[pscustomobject]@{tag_name=$tag;draft=$false;prerelease=$true;published_at='2026-10-28T00:00:00Z';assets=@(
            @($name,'SHA256SUMS.txt','release-manifest.json') | ForEach-Object {[pscustomobject]@{name=$_;browser_download_url="https://github.com/qingtian-4/TaskPulse/releases/download/$tag/$_"}}
        )}
        function Invoke-RestMethod { param($Uri,$Headers,$TimeoutSec)
            if($Uri -cne "https://api.github.com/repos/qingtian-4/TaskPulse/releases/tags/$tag"){throw 'Unexpected API location'}
            return $script:release
        }
        function Invoke-WebRequest {param($Uri,$OutFile,$TimeoutSec,[switch]$UseBasicParsing)
            $file=[IO.Path]::GetFileName($OutFile)
            if($Uri -cne "https://github.com/qingtian-4/TaskPulse/releases/download/$tag/$file"){throw 'Unexpected asset location'}
            Copy-Item -LiteralPath (Join-Path $response $file) -Destination $OutFile
        }
        $downloads=Join-Path $temp 'downloads'
        Invoke-TaskPulseInstall -Version $Version -Prerelease $true -DownloadOnly $true -OutputDirectory $downloads -Language en
        $verified=Test-TaskPulseDownload (Join-Path $downloads "TaskPulse-$tag") $tag
        $hash=(Get-TaskPulseProbeHash $verified)
        if($hash -cne (Get-TaskPulseProbeHash (Join-Path $assetRoot $name))){throw 'Downloaded package changed'}
        $passed=$true
    } finally {
        @{passed=$passed;source_commit=$preview.commit;os=[Environment]::OSVersion.VersionString;architecture='x64';installation_method='production-bootstrap-download-validation';script_sha256=(Get-TaskPulseProbeHash (Join-Path $PSScriptRoot '..\install.ps1'));package_sha256=$hash;installer_execution='separate installer-smoke.ps1 job step';anonymous_public_download_verified=$false} | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $evidence 'bootstrap.json') -Encoding UTF8
        Remove-Item -LiteralPath $temp -Recurse -Force
    }
}
if($MyInvocation.InvocationName -ne '.') {
    Invoke-TaskPulseBootstrapProbe -Assets $Assets -Version $Version -Evidence (Join-Path (Get-Location).Path 'dist\installer-smoke')
}
