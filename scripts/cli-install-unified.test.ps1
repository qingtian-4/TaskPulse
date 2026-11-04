$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot '..\install.ps1')
function Assert([bool]$Condition,[string]$Message) { if(-not $Condition){throw $Message} }
function Reject([scriptblock]$Action) {
    $failed=$false
    try { & $Action | Out-Null } catch { $failed=$true }
    Assert $failed 'Invalid unified download accepted'
}
$root=Join-Path ([IO.Path]::GetTempPath()) ('tp-unified-'+[Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($root)
$originalArch=$env:PROCESSOR_ARCHITECTURE
$originalNative=$env:PROCESSOR_ARCHITEW6432
try {
    $tag='v1.1.1'
    $hash='128b1c2c6cbdd79a0cfa3bf69f8b2a7ecb7b07b64c22ad7791f9dff553aec477'
    foreach($arch in @('x64','arm64')) {
        $name="TaskPulse_1.1.1_windows_${arch}_setup.exe"
        [IO.File]::WriteAllBytes((Join-Path $root $name),[byte[]](77,90,1,2,3,4))
        $platform=@{version='1.1.1';tag=$tag;platform='windows';architecture=$arch;commit=('a'*40);assets=@($name)}
        $manifest=@{format='taskpulse-release-v2';version='1.1.1';tag=$tag;build_kind='release';commit=('a'*40);platforms=@{windows=@{}};assets=@{}}
        $manifest.platforms.windows[$arch]=$platform
        $manifest.assets[$name]=$hash
        function Save { $manifest | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $root 'release-manifest.json') }
        "$hash  $name`n$('f'*64)  unrelated.dmg" | Set-Content -LiteralPath (Join-Path $root 'SHA256SUMS.txt')
        Save
        Assert ((Test-TaskPulseDownload $root $tag $arch) -eq (Join-Path $root $name)) 'Matching unified download rejected'
        $platform.commit='b'*40;Save;Reject {Test-TaskPulseDownload $root $tag $arch};$platform.commit='a'*40
        $manifest.assets[$name]='f'*64;Save;Reject {Test-TaskPulseDownload $root $tag $arch};$manifest.assets[$name]=$hash
        $manifest.build_kind='pr-preview';Save;Reject {Test-TaskPulseDownload $root $tag $arch};$manifest.build_kind='release'
        $manifest.format='unknown';Save;Reject {Test-TaskPulseDownload $root $tag $arch};$manifest.format='taskpulse-release-v2'
    }
    $env:PROCESSOR_ARCHITECTURE='x86';$env:PROCESSOR_ARCHITEW6432='ARM64'
    Assert ((Get-TaskPulseArchitecture) -eq 'arm64') 'WOW native ARM64 not selected'
    $env:PROCESSOR_ARCHITEW6432='AMD64'
    Assert ((Get-TaskPulseArchitecture) -eq 'x64') 'WOW native x64 not selected'
    $env:PROCESSOR_ARCHITEW6432='';$env:PROCESSOR_ARCHITECTURE='ARM64'
    Assert ((Get-TaskPulseArchitecture) -eq 'arm64') 'Native ARM64 not selected'
    $env:PROCESSOR_ARCHITECTURE='x86'
    Reject {Get-TaskPulseArchitecture}
    Write-Output 'PASS: unified manifests and native architecture; no installer executed.'
} finally {
    $env:PROCESSOR_ARCHITECTURE=$originalArch
    $env:PROCESSOR_ARCHITEW6432=$originalNative
    Remove-Item -LiteralPath $root -Recurse -Force
}
