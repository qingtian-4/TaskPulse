$ErrorActionPreference='Stop'
class ReleaseLimitException : System.Exception {
    [object]$Response
    ReleaseLimitException([int]$Status,[string]$Remaining) : base('GitHub request rejected') {
        $this.Response=@{StatusCode=$Status;Headers=@{'X-RateLimit-Remaining'=$Remaining}}
    }
}
. (Join-Path $PSScriptRoot '..\install.ps1')
# Reproduce hosts where this optional convenience command is unavailable.
function Get-FileHash { throw 'Get-FileHash unavailable in this test host' }
function Assert([bool]$Condition,[string]$Message) { if(-not $Condition){throw $Message} }
function Reject([scriptblock]$Action,[string]$Message) {
    $rejected=$false
    try { & $Action | Out-Null } catch { $rejected=$true }
    Assert $rejected $Message
}
$root=Join-Path ([IO.Path]::GetTempPath()) ('taskpulse-cli-test-'+[Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($root)
try {
    $tag='v0.2.0-alpha.8';$name='TaskPulse_0.2.0-alpha.8_windows_x64_setup.exe'
    $payload=Join-Path $root $name
    [IO.File]::WriteAllBytes($payload,[byte[]](77,90,1,2,3,4))
    # Independently precomputed for the fixed six fixture bytes above.
    $sum='128b1c2c6cbdd79a0cfa3bf69f8b2a7ecb7b07b64c22ad7791f9dff553aec477'
    $manifest=@{version='0.2.0-alpha.8';tag=$tag;platform='windows';architecture='x64';commit=('a'*40);assets=@($name)}
    $manifest | ConvertTo-Json | Set-Content (Join-Path $root 'release-manifest.json')
    "$sum  $name" | Set-Content (Join-Path $root 'SHA256SUMS.txt')
    Assert ((Test-TaskPulseDownload $root $tag) -eq $payload) 'Matching download rejected'
    "$sum  $name`n$sum  $name" | Set-Content (Join-Path $root 'SHA256SUMS.txt')
    Reject {Test-TaskPulseDownload $root $tag} 'Duplicate checksum accepted'
    'malformed' | Set-Content (Join-Path $root 'SHA256SUMS.txt')
    Reject {Test-TaskPulseDownload $root $tag} 'Malformed checksum accepted'
    "$sum  $name" | Set-Content (Join-Path $root 'SHA256SUMS.txt')
    [IO.File]::WriteAllBytes($payload,[byte[]](77,90,9))
    Reject {Test-TaskPulseDownload $root $tag} 'Corrupt installer accepted'
    [IO.File]::WriteAllBytes($payload,[byte[]](77,90,1,2,3,4))
    foreach($field in @('version','tag','platform','architecture','commit')) {
        $original=$manifest[$field];$manifest[$field]='mismatch'
        $manifest | ConvertTo-Json | Set-Content (Join-Path $root 'release-manifest.json')
        Reject {Test-TaskPulseDownload $root $tag} "Mismatched $field accepted"
        $manifest[$field]=$original
    }
    $manifest | ConvertTo-Json | Set-Content (Join-Path $root 'release-manifest.json')
    Reject {Test-TaskPulseDownload $root '../escape'} 'Traversal tag accepted'
    $manifest['build_kind']='pr-preview'
    $manifest | ConvertTo-Json | Set-Content (Join-Path $root 'release-manifest.json')
    Reject {Test-TaskPulseDownload $root $tag} 'Preview accepted as a public release'
    $manifest.Remove('build_kind')
    $manifest | ConvertTo-Json | Set-Content (Join-Path $root 'release-manifest.json')
    $script:release=[pscustomobject]@{tag_name=$tag;draft=$false;prerelease=$true;published_at='2026-10-28T00:00:00Z';assets=@(
        @($name,'SHA256SUMS.txt','release-manifest.json') | ForEach-Object { [pscustomobject]@{name=$_;browser_download_url="https://github.com/qingtian-4/TaskPulse/releases/download/$tag/$_"} }
    )}
    Assert ((Get-TaskPulseAssetUrl $script:release $name) -like "https://github.com/*/$name") 'Asset selection failed'
    $url=$script:release.assets[0].browser_download_url
    $script:release.assets[0].browser_download_url='https://example.com/untrusted.exe'
    Reject {Get-TaskPulseAssetUrl $script:release $name} 'Off-repository download accepted'
    $script:release.assets[0].browser_download_url=$url
    Reject {Get-TaskPulseAssetUrl $script:release 'missing.exe'} 'Missing asset accepted'
    $assets=$script:release.assets
    $script:release.assets=@($assets)+@($assets[0])
    Reject {Get-TaskPulseAssetUrl $script:release $name} 'Duplicate asset accepted'
    $script:release.assets=$assets
    function Invoke-RestMethod {
        param($Uri,$Headers,$TimeoutSec)
        $script:lastUri=$Uri
        if($script:apiLimited){
            $failure=[ReleaseLimitException]::new(403,$script:apiRemaining)
            if($script:coreResponse){
                $failure.Response=[Net.Http.HttpResponseMessage]::new([Net.HttpStatusCode]::Forbidden)
                [void]$failure.Response.Headers.TryAddWithoutValidation('X-RateLimit-Remaining',$script:apiRemaining)
            }
            throw $failure
        }
        return $script:release
    }
    Reject {Get-TaskPulseRelease '' $false} 'Stable selection accepted prerelease'
    Assert ((Get-TaskPulseRelease '' $true).tag_name -eq $tag) 'Explicit prerelease selection failed'
    Assert ($script:lastUri -like '*?per_page=100') 'Prerelease used stable latest endpoint'
    Assert ((Get-TaskPulseRelease '0.2.0-alpha.8' $false).tag_name -eq $tag) 'Explicit version rejected'
    Reject {Get-TaskPulseRelease '0.2.0-alpha.7' $false} 'Wrong tag accepted'
    Reject {Get-TaskPulseRelease '../bad' $true} 'Invalid version accepted'
    $script:release.draft=$true
    Reject {Get-TaskPulseRelease '0.2.0-alpha.8' $true} 'Draft accepted'
    Reject {Get-TaskPulseRelease '' $true} 'Draft-only list accepted'
    $script:release.draft=$false
    function Invoke-WebRequest {
        param($Uri,$Headers,$Method,$TimeoutSec,$MaximumRedirection,[switch]$UseBasicParsing)
        Assert ($Method -eq 'Head') 'Release fallback must only inspect the public page'
        $script:webRequests++
        $final=if($script:webRedirect){$script:webRedirect}elseif($Uri.EndsWith('/latest')){'https://github.com/qingtian-4/TaskPulse/releases/tag/v1.0.0'}else{$Uri}
        if($script:coreResponse){return [pscustomobject]@{BaseResponse=[pscustomobject]@{RequestMessage=[pscustomobject]@{RequestUri=[uri]$final}}}}
        return [pscustomobject]@{BaseResponse=[pscustomobject]@{ResponseUri=[uri]$final}}
    }
    $script:apiLimited=$true;$script:apiRemaining='0';$script:webRequests=0
    Assert ((Get-TaskPulseRelease '' $false).tag_name -eq 'v1.0.0') 'API limit prevented a verified public stable release'
    Add-Type -AssemblyName System.Net.Http
    $script:coreResponse=$true
    Assert ((Get-TaskPulseRelease '' $false).tag_name -eq 'v1.0.0') 'HttpClient header/response shape was not supported'
    $script:coreResponse=$false
    Assert ((Get-TaskPulseRelease '0.2.0-alpha.8' $false).tag_name -eq $tag) 'Explicit public tag fallback failed'
    Reject {Get-TaskPulseRelease '' $true} 'Rate-limited prerelease discovery silently selected stable'
    $script:webRedirect='https://example.com/qingtian-4/TaskPulse/releases/tag/v1.0.0'
    Reject {Get-TaskPulseRelease '' $false} 'Untrusted release-page redirect accepted'
    $script:webRedirect='https://github.com/another/repo/releases/tag/v1.0.0'
    Reject {Get-TaskPulseRelease '' $false} 'Another repository accepted'
    $script:webRedirect='https://github.com/qingtian-4/TaskPulse/releases/tag/v1.0.0-alpha.1'
    Reject {Get-TaskPulseRelease '' $false} 'Stable fallback accepted a prerelease'
    $script:webRedirect=$null;$beforeWeb=$script:webRequests;$script:apiRemaining='1'
    Reject {Get-TaskPulseRelease '' $false} 'Non-rate-limit API denial was ignored'
    Assert ($script:webRequests -eq $beforeWeb) 'Permission denial attempted a network workaround'
    $script:apiLimited=$false
    function Invoke-WebRequest {
        param($Uri,$OutFile,$Headers,$TimeoutSec,[switch]$UseBasicParsing)
        $script:assetRequests++
        if($script:networkFailures -gt 0){$script:networkFailures--;throw [Net.WebException]::new('Fixture connection failure',[Net.WebExceptionStatus]::ConnectFailure)}
        Copy-Item -LiteralPath (Join-Path $root ([IO.Path]::GetFileName($OutFile))) -Destination $OutFile
        if ($script:racePath -and $Uri.EndsWith('/release-manifest.json')) {
            [void][IO.Directory]::CreateDirectory($script:racePath)
            'previous download' | Set-Content (Join-Path $script:racePath 'sentinel.txt')
        }
    }
    function Start-Process { throw 'Offline smoke must never start an installer' }
    foreach($lang in @('en','zh-CN')) {
        $out=Join-Path $root $lang
        Invoke-TaskPulseInstall -Version '0.2.0-alpha.8' -DownloadOnly $true -OutputDirectory $out -Language $lang
        Assert (Test-Path (Join-Path $out "TaskPulse-$tag\$name")) 'Verified download was not saved'
        Reject {Invoke-TaskPulseInstall -Version '0.2.0-alpha.8' -DownloadOnly $true -OutputDirectory $out -Language $lang} 'Existing download overwritten'
    }
    $relativeRoot=Join-Path $root 'relative'
    [void][IO.Directory]::CreateDirectory($relativeRoot)
    Push-Location $relativeRoot
    try {
        Invoke-TaskPulseInstall -Version '0.2.0-alpha.8' -DownloadOnly $true -OutputDirectory '.\downloads' -Language en
        Assert (Test-Path -LiteralPath (Join-Path $relativeRoot "downloads\TaskPulse-$tag\$name")) 'Relative path ignored PowerShell location'
        Reject {Invoke-TaskPulseInstall -Version '0.2.0-alpha.8' -DownloadOnly $true -OutputDirectory 'HKCU:\Software\TaskPulseTest' -Language en} 'Non-filesystem output path accepted'
    } finally { Pop-Location }
    $raceRoot=Join-Path $root 'concurrent'
    $script:racePath=Join-Path $raceRoot "TaskPulse-$tag"
    Reject {Invoke-TaskPulseInstall -Version '0.2.0-alpha.8' -DownloadOnly $true -OutputDirectory $raceRoot -Language en} 'Concurrent destination was overwritten or nested'
    Assert (@(Get-ChildItem -LiteralPath $script:racePath).Count -eq 1) 'Concurrent destination was modified'
    Assert ((Get-Content (Join-Path $script:racePath 'sentinel.txt')) -eq 'previous download') 'Concurrent data lost'
    Assert (@(Get-ChildItem -LiteralPath $raceRoot).Count -eq 1) 'Owned staging leaked after conflict'
    $script:racePath=$null
    $script:networkFailures=1;$script:assetRequests=0
    Invoke-TaskPulseInstall -Version '0.2.0-alpha.8' -DownloadOnly $true -OutputDirectory (Join-Path $root 'retry') -Language en
    Assert ($script:assetRequests -eq 4) 'Transient failure did not retry exactly once'
    $script:networkFailures=3;$failure=''
    try{Invoke-TaskPulseInstall -Version '0.2.0-alpha.8' -DownloadOnly $true -OutputDirectory (Join-Path $root 'failed') -Language en}catch{$failure=$_.Exception.Message}
    Assert ($failure.Contains($name) -and $failure.Contains("https://github.com/qingtian-4/TaskPulse/releases/download/$tag/$name")) 'Failure omitted the exact asset and official URL'
    Assert (@(Get-ChildItem (Join-Path $root 'failed')).Count -eq 0) 'Failed download left staging data'
    $script:networkFailures=0
    # Native preview probe must preserve its Version parameter while dot-sourcing
    # the production script. Downloads only: fixture MZ bytes are never executed.
    . (Join-Path $PSScriptRoot 'windows-cli-smoke.ps1') -Assets $root -Version '0.2.0-alpha.8'
    $manifest['tag']=$null
    $manifest['build_kind']='pr-preview'
    $manifest | ConvertTo-Json | Set-Content (Join-Path $root 'release-manifest.json')
    $probeEvidence=Join-Path $root 'probe-evidence'
    Invoke-TaskPulseBootstrapProbe -Assets $root -Version '0.2.0-alpha.8' -Evidence $probeEvidence
    $probe=Get-Content -LiteralPath (Join-Path $probeEvidence 'bootstrap.json') -Raw | ConvertFrom-Json
    Assert ($probe.passed -and $probe.source_commit -eq ('a'*40)) 'Native preview bootstrap probe failed'
    Write-Output 'PASS: release selection, draft rejection, asset origin, manifest, checksum, corruption, download-only and bilingual output; no installer executed.'
} finally { Remove-Item -LiteralPath $root -Recurse -Force }
