param(
    [switch]$Frontend
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$manifest = Join-Path $repoRoot 'src-tauri\Cargo.toml'
$env:CARGO_TARGET_DIR = Join-Path $repoRoot 'src-tauri\target\codex-check'
$mingwBin = 'C:\tools\mingw64\bin'
if (Test-Path -LiteralPath $mingwBin) {
    $env:PATH = "$mingwBin;$env:USERPROFILE\.cargo\bin;$env:PATH"
    $env:CC = 'gcc'
    $env:CXX = 'g++'
    $env:RUSTFLAGS = '-C link-self-contained=yes'
}

Push-Location $repoRoot
try {
    cargo fmt --manifest-path $manifest -- --check
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    cargo clippy --manifest-path $manifest --bin agent-tray -- -D warnings
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    cargo test --manifest-path $manifest --bin agent-tray
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    cargo build --manifest-path $manifest --bin agent-tray
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    if ($Frontend) {
        $bun = Join-Path $env:LOCALAPPDATA 'hermes\node\bun.cmd'
        if (-not (Test-Path -LiteralPath $bun)) { $bun = 'bun.cmd' }
        & $bun run check
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        & $bun run build
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
}
finally {
    Pop-Location
}
