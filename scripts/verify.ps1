param(
    [switch]$Frontend,
    [string]$TargetDir
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$manifest = Join-Path $repoRoot 'src-tauri\Cargo.toml'
$taskPulseTargetRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot 'src-tauri\target'))
$taskPulseVerifyTarget = if ($TargetDir) { [IO.Path]::GetFullPath((Join-Path $repoRoot $TargetDir)) } else { Join-Path $taskPulseTargetRoot 'codex-check' }
if ($taskPulseVerifyTarget -ne $taskPulseTargetRoot -and -not $taskPulseVerifyTarget.StartsWith($taskPulseTargetRoot + [IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) { throw 'Verification target must stay under src-tauri/target.' }
$taskPulseVerifyExe = Join-Path $taskPulseVerifyTarget 'debug\agent-tray.exe'
if (Get-CimInstance Win32_Process -Filter "Name = 'agent-tray.exe'" | Where-Object { $_.ExecutablePath -eq $taskPulseVerifyExe }) { throw 'Chosen verification target is running. Select another target instead of replacing the live executable.' }
$env:CARGO_TARGET_DIR = $taskPulseVerifyTarget
$mingwBin = 'C:\tools\mingw64\bin'
if (Test-Path -LiteralPath $mingwBin) {
    $env:PATH = "$mingwBin;$env:USERPROFILE\.cargo\bin;$env:PATH"
    $env:CC = 'gcc'
    $env:CXX = 'g++'
    $env:RUSTFLAGS = '-C link-self-contained=yes'
}

Push-Location $repoRoot
try {
    # Build frontend before Rust embeds it; compiling Rust first can validate an
    # old UI while the subsequent frontend step succeeds on different assets.
    if ($Frontend) {
        & powershell -NoProfile -ExecutionPolicy Bypass -File scripts/cache-tools.test.ps1
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        $bun = Join-Path $env:LOCALAPPDATA 'hermes\node\bun.cmd'
        if (-not (Test-Path -LiteralPath $bun)) { $bun = 'bun.cmd' }
        & $bun run audit:frontend
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        & $bun run check
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        & $bun run test:docs
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        & $bun run test:repo
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        # sqlite3 context managers commit/rollback but do not close handles;
        # these offline helpers must also pass on Windows file locking.
        $taskPulsePython = if (Get-Command python3.11.exe -ErrorAction SilentlyContinue) { 'python3.11.exe' } else { 'python' }
        foreach ($fixture in @('scripts/test-wsl-discovery.py', 'scripts/test-codex-metadata.py', 'scripts/wsl-metadata.test.py', 'scripts/test-codex-observer.py')) {
            & $taskPulsePython $fixture
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        }
        & $bun run test:ui-model
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        & $bun run release:check
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        & $bun run test:release
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        & $bun run test:security
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        & $bun run test:integration-contract
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        & $bun run test:smoke-cleanup
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        & $bun run build
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
    cargo fmt --manifest-path $manifest -- --check
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    cargo clippy --manifest-path $manifest --bin agent-tray -- -D warnings
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    cargo test --manifest-path $manifest --bin agent-tray
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    cargo build --manifest-path $manifest --bin agent-tray --features custom-protocol
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    if ($Frontend) {
        & $bun scripts/integration-bridge-smoke.ts --exe $taskPulseVerifyExe
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        & $taskPulsePython scripts/mcp-smoke.py --exe $taskPulseVerifyExe --out (Join-Path $taskPulseVerifyTarget 'mcp-smoke.json')
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }

}
finally {
    Pop-Location
}
