$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or -not $env:RUNNER_TEMP -or -not $env:GITHUB_ENV) {
    throw 'Run only on a disposable GitHub-hosted runner'
}

# The ARM64 hosted image does not provide WinGet. Use the upstream
# portable binary in runner-owned storage without installing it.
$directory = Join-Path $env:RUNNER_TEMP ("taskpulse-gsudo-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $directory | Out-Null
$archive = Join-Path $directory 'gsudo.portable.zip'
curl.exe --silent --show-error --fail --location --proto '=https' --proto-redir '=https' --retry 2 --retry-all-errors --retry-max-time 60 --connect-timeout 12 --max-time 30 --output "$archive" 'https://github.com/gerardog/gsudo/releases/download/v2.6.1/gsudo.portable.zip'
if ($LASTEXITCODE -ne 0) { throw 'Could not download ARM64 CI de-elevation helper' }
$expected = '21130bf178d7b9891207f00bff56f05b6b363ce9cec7b2d084e45fb12ee51f44'
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $expected) { throw 'ARM64 CI de-elevation helper checksum mismatch' }
$gsudoPath = Join-Path $directory 'gsudo.exe'
$zip = [IO.Compression.ZipFile]::OpenRead($archive)
try {
  $entry = $zip.GetEntry('arm64/gsudo.exe')
  if (-not $entry) { throw 'Missing ARM64 de-elevation executable in verified archive' }
  [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $gsudoPath)
} finally {
  $zip.Dispose()
}
Remove-Item -LiteralPath $archive
if (-not (Test-Path -LiteralPath $gsudoPath)) { throw 'Missing CI de-elevation helper' }
"GSUDO_EXE=$gsudoPath" >> $env:GITHUB_ENV
$ciAccount = [Security.Principal.WindowsIdentity]::GetCurrent().Name
icacls "$env:GITHUB_WORKSPACE" /grant "${ciAccount}:(OI)(CI)M"
if ($LASTEXITCODE -ne 0) { throw 'Could not grant the runner account access to acceptance evidence' }
