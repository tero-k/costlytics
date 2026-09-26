# Runs a local release build of Costlytics in the browser, without the desktop
# app: the dev/test API harness (costlytics-api) on :3000 and the built
# frontend (via `vite preview`, which proxies /api to the backend) on :4173,
# against the synthetic fixtures.
#
#   .\run-local.ps1          # run an existing build
#   .\run-local.ps1 -Build   # build backend, fixtures and frontend first
#
# Settings changes go to a scratch copy of config/example.toml
# (target\local-settings.toml), so the checked-in config stays clean.
# Press Ctrl+C to stop; the backend is stopped too.

param([switch]$Build)

$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$backendExe = Join-Path $root 'target\release\costlytics-api.exe'
$fixturesExe = Join-Path $root 'target\release\generate-fixtures.exe'

function Invoke-Checked([string]$what, [scriptblock]$cmd) {
    Write-Host "==> $what"
    # cargo/npm write progress to stderr; judge success by exit code only
    # (with 'Stop', Windows PowerShell 5.1 turns redirected stderr into errors).
    $ErrorActionPreference = 'Continue'
    & $cmd
    if ($LASTEXITCODE -ne 0) { throw "$what failed (exit $LASTEXITCODE)" }
}

Push-Location $root
try {
    if ($Build) {
        Invoke-Checked 'Building backend' { cargo build --release -p api -p data }
        Invoke-Checked 'Generating fixtures' { & $fixturesExe }
        Push-Location web
        try {
            if (-not (Test-Path node_modules)) { Invoke-Checked 'Installing frontend dependencies' { npm ci } }
            Invoke-Checked 'Building frontend' { npm run build }
        }
        finally { Pop-Location }
    }

    if (-not (Test-Path $backendExe)) { throw "Missing $backendExe - run: .\run-local.ps1 -Build" }
    if (-not (Test-Path 'web\dist\index.html')) { throw 'Missing web\dist - run: .\run-local.ps1 -Build' }
    if (-not (Test-Path 'fixtures\demo')) { throw 'Missing fixtures - run: .\run-local.ps1 -Build' }
}
finally { Pop-Location }

$settings = Join-Path $root 'target\local-settings.toml'
if (-not (Test-Path $settings)) { Copy-Item (Join-Path $root 'config\example.toml') $settings }

# Relative source paths in the config resolve against the working directory.
$env:COSTLYTICS_CONFIG = $settings
$backend = Start-Process -FilePath $backendExe -WorkingDirectory $root -PassThru -NoNewWindow
$pushed = $false
try {
    $ready = $false
    for ($i = 0; $i -lt 60; $i++) {
        if ($backend.HasExited) { throw "Backend exited with code $($backend.ExitCode) (is port 3000 already in use?)" }
        try { Invoke-RestMethod 'http://127.0.0.1:3000/api/v1/health' | Out-Null; $ready = $true; break }
        catch { Start-Sleep -Milliseconds 500 }
    }
    if (-not $ready) { throw 'Backend did not become ready on http://127.0.0.1:3000' }

    Write-Host 'Costlytics: http://localhost:4173  (sample data is August 2026 - pick the local-demo source and set the date range to it)'
    Start-Process 'http://localhost:4173'
    Push-Location (Join-Path $root 'web')
    $pushed = $true
    npx vite preview --port 4173 --strictPort
}
finally {
    if ($pushed) { Pop-Location }
    Remove-Item Env:COSTLYTICS_CONFIG -ErrorAction SilentlyContinue
    if (-not $backend.HasExited) { Stop-Process -Id $backend.Id -Force }
}
