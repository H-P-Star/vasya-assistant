[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    & (Join-Path $PSScriptRoot 'prepare-bundle.ps1')
    npm ci
    if ($LASTEXITCODE -ne 0) { throw 'npm ci failed' }
    cargo test --manifest-path src-tauri/Cargo.toml --lib --locked
    if ($LASTEXITCODE -ne 0) { throw 'Rust tests failed' }
    npm run tauri build
    if ($LASTEXITCODE -ne 0) { throw 'Windows build failed' }
    Write-Host 'Installer: src-tauri\target\release\bundle\nsis'
} finally { Pop-Location }
