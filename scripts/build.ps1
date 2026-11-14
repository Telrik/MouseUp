param([string]$OutputDirectory = 'dist-single')
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    if (-not (Test-Path -LiteralPath 'assets\yolomouse\Cursors\Arrow\Blue.cur')) {
        throw 'Original cursor files are missing. Run scripts\extract-cursors.ps1 with your YoloMouse MSI first.'
    }
    foreach ($target in @('x86_64-pc-windows-msvc','i686-pc-windows-msvc')) {
        cargo build --release --target $target -p mouseup-hook -p mouseup-hook-host
        if ($LASTEXITCODE -ne 0) { throw "Hook build failed: $target" }
    }
    $previousBundle = $env:MOUSEUP_BUNDLE
    try {
        $env:MOUSEUP_BUNDLE = '1'
        cargo build --release -p mouseup
        if ($LASTEXITCODE -ne 0) { throw 'Application build failed' }
    } finally { $env:MOUSEUP_BUNDLE = $previousBundle }
    New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
    Copy-Item target\release\mouseup.exe (Join-Path $OutputDirectory 'mouseup.exe') -Force
    Write-Host "Built $OutputDirectory\mouseup.exe"
} finally { Pop-Location }
