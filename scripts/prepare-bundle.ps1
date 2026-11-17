# Build-time only. End users never execute this script.
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) { throw 'CMake is required on the build machine, not on the end-user PC' }
$root = Split-Path $PSScriptRoot -Parent
$lock = Get-Content (Join-Path $PSScriptRoot 'speech-assets.json') -Raw | ConvertFrom-Json
$temporary = Join-Path ([IO.Path]::GetTempPath()) ('vasya-bundle-' + [Guid]::NewGuid().ToString('N'))
$destination = Join-Path $root 'src-tauri\resources\speech'
New-Item -ItemType Directory -Force -Path $temporary | Out-Null
try {
    foreach ($name in @('vosk', 'model', 'whisper_source')) {
        $asset = $lock.$name
        $archive = Join-Path $temporary "$name.zip"
        Invoke-WebRequest -UseBasicParsing -Uri $asset.url -OutFile $archive
        if ((Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $asset.sha256) {
            throw "SHA256 mismatch for $name. Refusing to bundle."
        }
        Expand-Archive -LiteralPath $archive -DestinationPath (Join-Path $temporary $name)
    }
    $whisperModel = Join-Path $temporary 'ggml-base.bin'
    Invoke-WebRequest -UseBasicParsing -Uri $lock.whisper_model.url -OutFile $whisperModel
    if ((Get-FileHash $whisperModel -Algorithm SHA256).Hash.ToLowerInvariant() -ne $lock.whisper_model.sha256) { throw 'SHA256 mismatch for Whisper model' }
    $staged = Join-Path $temporary 'speech'
    New-Item -ItemType Directory -Force -Path (Join-Path $staged 'vosk'), (Join-Path $staged 'models'), (Join-Path $staged 'whisper') | Out-Null
    Get-ChildItem (Join-Path $temporary 'vosk') -Recurse -Filter '*.dll' | Copy-Item -Destination (Join-Path $staged 'vosk')
    Copy-Item (Join-Path $temporary 'model\vosk-model-small-ru-0.22') -Destination (Join-Path $staged 'models') -Recurse
    # Static CRT: do not require Visual C++ Redistributable on the end user's PC.
    $source = Join-Path $temporary 'whisper_source\whisper.cpp-1.8.2'
    foreach ($variant in @('fast','baseline')) {
        $build = Join-Path $temporary "whisper-build-$variant"
        $simd = if ($variant -eq 'fast') { 'ON' } else { 'OFF' }
        & cmake -S $source -B $build -A x64 -DBUILD_SHARED_LIBS=OFF -DCMAKE_POLICY_DEFAULT_CMP0091=NEW -DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded -DGGML_STATIC=ON -DGGML_OPENMP=OFF -DGGML_NATIVE=OFF "-DGGML_SSE42=$simd" "-DGGML_BMI2=$simd" "-DGGML_AVX=$simd" "-DGGML_AVX2=$simd" "-DGGML_FMA=$simd" "-DGGML_F16C=$simd" -DWHISPER_BUILD_TESTS=OFF -DWHISPER_BUILD_SERVER=OFF
        if ($LASTEXITCODE -ne 0) { throw 'Whisper configure failed' }
        & cmake --build $build --config Release --target whisper-cli --parallel 4
        if ($LASTEXITCODE -ne 0) { throw 'Whisper build failed' }
        $cli = Get-ChildItem $build -Recurse -Filter whisper-cli.exe | Select-Object -First 1
        if (-not $cli) { throw 'Whisper executable missing after build' }
        $filename = if ($variant -eq 'fast') { 'whisper-cli.exe' } else { 'whisper-cli-baseline.exe' }
        Copy-Item $cli.FullName -Destination (Join-Path $staged "whisper\$filename")
        $dumpbin = Get-ChildItem "${env:ProgramFiles}\Microsoft Visual Studio\2022\*\VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe" -ErrorAction SilentlyContinue | Select-Object -Last 1
        if ($dumpbin) {
            $imports = & $dumpbin.FullName /DEPENDENTS $cli.FullName
            if ($imports -match 'VCRUNTIME|MSVCP[0-9]|ggml\.dll|whisper\.dll') { throw 'Whisper must include its runtime and libraries statically' }
        }
    }
    Copy-Item $whisperModel -Destination (Join-Path $staged 'models\ggml-base.bin')
    foreach ($file in @('whisper\whisper-cli-baseline.exe','whisper\whisper-cli.exe','models\ggml-base.bin','vosk\libvosk.dll','vosk\libgcc_s_seh-1.dll','vosk\libstdc++-6.dll','vosk\libwinpthread-1.dll','models\vosk-model-small-ru-0.22\am\final.mdl')) {
        if (-not (Test-Path (Join-Path $staged $file))) { throw "Missing bundled component: $file" }
    }
    New-Item -ItemType Directory -Force -Path $destination | Out-Null
    Copy-Item (Join-Path $staged '*') -Destination $destination -Recurse -Force
} finally {
    Remove-Item -LiteralPath $temporary -Recurse -Force
}
