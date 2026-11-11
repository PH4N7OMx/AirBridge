param([switch]$Deploy)
$ErrorActionPreference = 'Stop'
$root = Split-Path (Split-Path $PSScriptRoot)
# Do not embed the developer's Windows profile/workspace in panic locations.
# Encoded flags keep paths with spaces as a single rustc argument.
$originalEncodedFlags = $env:CARGO_ENCODED_RUSTFLAGS
[string[]]$flags = @()
if ($originalEncodedFlags) { $flags = @($originalEncodedFlags.Split([char]31)) }
elseif ($env:RUSTFLAGS) { $flags = @($env:RUSTFLAGS -split '\s+' | Where-Object { $_ }) }
if ($env:USERPROFILE) { $flags += "--remap-path-prefix=$env:USERPROFILE=/build/home" }
$flags += "--remap-path-prefix=$root=/source/airbridge"
$env:CARGO_ENCODED_RUSTFLAGS = $flags -join [char]31
$sdk = Join-Path $PSScriptRoot 'vendor\ffmpeg-airbridge-9.0.2'
if (-not (Test-Path -LiteralPath "$sdk\include\libavcodec\avcodec.h")) { throw 'FFmpeg development package missing. See README.md.' }
$env:FFMPEG_DIR = $sdk
$env:LIBCLANG_PATH = Join-Path $PSScriptRoot 'tools\clang\clang\native'
Push-Location $PSScriptRoot
try {
    cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Rust build failed' }
    if ($Deploy) {
        Copy-Item -LiteralPath "$sdk\bin\avcodec-63.dll","$sdk\bin\avformat-63.dll","$sdk\bin\avutil-61.dll","$sdk\bin\swscale-10.dll","$sdk\bin\swresample-7.dll" -Destination $root
        Copy-Item -LiteralPath 'target\release\AirBridge.exe' -Destination "$root\AirBridge.exe"
        $runtimeFiles = @('AirBridge.exe','avcodec-63.dll','avformat-63.dll','avutil-61.dll','swscale-10.dll','swresample-7.dll')
        $manifest = foreach ($runtimeFile in $runtimeFiles) {
            $hash = (Get-FileHash -LiteralPath (Join-Path $root $runtimeFile) -Algorithm SHA256).Hash.ToLowerInvariant()
            "$hash  $runtimeFile"
        }
        [IO.File]::WriteAllLines((Join-Path $PSScriptRoot 'build-manifest.sha256'),$manifest)
    }
} finally {
    Pop-Location
    $env:CARGO_ENCODED_RUSTFLAGS = $originalEncodedFlags
}
