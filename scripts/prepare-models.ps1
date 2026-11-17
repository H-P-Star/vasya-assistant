# Run from a normal PowerShell terminal; administrator rights are not required.
[CmdletBinding()]
param([string]$Destination = (Join-Path $env:LOCALAPPDATA 'Vasya'))
$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$temporary = Join-Path ([IO.Path]::GetTempPath()) ('vasya-models-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $temporary, $Destination | Out-Null
try {
    $dll = Join-Path $Destination 'vosk\libvosk.dll'
    if (-not (Test-Path $dll)) {
        Write-Host 'Downloading Vosk 0.3.45 for Windows x64...'
        $archive = Join-Path $temporary 'vosk.zip'
        Invoke-WebRequest -UseBasicParsing -Uri 'https://github.com/alphacep/vosk-api/releases/download/v0.3.45/vosk-win64-0.3.45.zip' -OutFile $archive
        Expand-Archive -LiteralPath $archive -DestinationPath (Join-Path $temporary 'vosk')
        $library = Get-ChildItem (Join-Path $temporary 'vosk') -Filter 'libvosk.dll' -Recurse | Select-Object -First 1
        if (-not $library) { throw 'libvosk.dll was not found in the official archive.' }
        New-Item -ItemType Directory -Force -Path (Join-Path $Destination 'vosk') | Out-Null
        Copy-Item (Join-Path $library.DirectoryName '*') -Destination (Join-Path $Destination 'vosk') -Recurse -Force
    }
    $modelRoot = Join-Path $Destination 'models'
    $model = Join-Path $modelRoot 'vosk-model-small-ru-0.22'
    if (-not (Test-Path (Join-Path $model 'am\final.mdl'))) {
        Write-Host 'Downloading the small Russian Vosk model (about 45 MB)...'
        $archive = Join-Path $temporary 'model.zip'
        Invoke-WebRequest -UseBasicParsing -Uri 'https://alphacephei.com/vosk/models/vosk-model-small-ru-0.22.zip' -OutFile $archive
        New-Item -ItemType Directory -Force -Path $modelRoot | Out-Null
        Expand-Archive -LiteralPath $archive -DestinationPath $modelRoot -Force
    }
    Write-Host "Ready. Vosk: $dll"
    Write-Host "Model: $model"
    Write-Host 'Open Vasya settings and save them to restart speech recognition.'
} finally {
    if (Test-Path $temporary) { Remove-Item -LiteralPath $temporary -Recurse -Force }
}
