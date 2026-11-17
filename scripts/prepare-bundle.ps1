# Build-time only. End users never execute this script.
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$lock = Get-Content (Join-Path $PSScriptRoot 'speech-assets.json') -Raw | ConvertFrom-Json
$temporary = Join-Path ([IO.Path]::GetTempPath()) ('vasya-bundle-' + [Guid]::NewGuid().ToString('N'))
$destination = Join-Path $root 'src-tauri\resources\speech'
New-Item -ItemType Directory -Force -Path $temporary | Out-Null
try {
    foreach ($name in @('vosk', 'model')) {
        $asset = $lock.$name
        $archive = Join-Path $temporary "$name.zip"
        Invoke-WebRequest -UseBasicParsing -Uri $asset.url -OutFile $archive
        if ((Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $asset.sha256) {
            throw "SHA256 mismatch for $name. Refusing to bundle."
        }
        Expand-Archive -LiteralPath $archive -DestinationPath (Join-Path $temporary $name)
    }
    $staged = Join-Path $temporary 'speech'
    New-Item -ItemType Directory -Force -Path (Join-Path $staged 'vosk'), (Join-Path $staged 'models') | Out-Null
    Get-ChildItem (Join-Path $temporary 'vosk') -Recurse -Filter '*.dll' | Copy-Item -Destination (Join-Path $staged 'vosk')
    Copy-Item (Join-Path $temporary 'model\vosk-model-small-ru-0.22') -Destination (Join-Path $staged 'models') -Recurse
    foreach ($file in @('vosk\libvosk.dll','vosk\libgcc_s_seh-1.dll','vosk\libstdc++-6.dll','vosk\libwinpthread-1.dll','models\vosk-model-small-ru-0.22\am\final.mdl')) {
        if (-not (Test-Path (Join-Path $staged $file))) { throw "Missing bundled component: $file" }
    }
    New-Item -ItemType Directory -Force -Path $destination | Out-Null
    Copy-Item (Join-Path $staged '*') -Destination $destination -Recurse -Force
} finally {
    Remove-Item -LiteralPath $temporary -Recurse -Force
}
