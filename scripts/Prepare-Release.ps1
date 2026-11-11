#requires -Version 7.0
param(
    [switch]$Installer,
    [string]$IsccPath
)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Split-Path $PSScriptRoot))
$privateMarkers = @($env:USERPROFILE,$root,$env:COMPUTERNAME) | Where-Object { $_ -and $_.Length -gt 5 }
function Assert-NoLocalIdentity([string]$path) {
    $bytes = [IO.File]::ReadAllBytes($path)
    $text = [Text.Encoding]::UTF8.GetString($bytes)
    $wide = [Text.Encoding]::Unicode.GetString($bytes)
    foreach ($marker in $privateMarkers) {
        foreach ($variant in @($marker,$marker.Replace('\','/'))) {
            if ($text.IndexOf($variant,[StringComparison]::OrdinalIgnoreCase) -ge 0 -or
                $wide.IndexOf($variant,[StringComparison]::OrdinalIgnoreCase) -ge 0) {
                throw "Local build-machine data detected in $([IO.Path]::GetFileName($path)). Rebuild with source/rust/build.ps1 or remove personal data before packaging."
            }
        }
    }
}
Assert-NoLocalIdentity (Join-Path $root 'AirBridge.exe')
$versionLine = Select-String -LiteralPath "$root/source/rust/Cargo.toml" -Pattern '^version = "([0-9]+\.[0-9]+\.[0-9]+)"$' | Select-Object -First 1
if (-not $versionLine) { throw 'Cannot read application version' }
$version = $versionLine.Matches[0].Groups[1].Value
$codecSdk = Join-Path $root 'source/rust/vendor/ffmpeg-airbridge-9.0.2'
$codecCache = Join-Path $root 'source/rust/tools/codec-build'
$codecStamp = Join-Path $codecSdk 'AIRBRIDGE-CODECS.json'
if (-not (Test-Path -LiteralPath $codecStamp)) { throw 'Build the pinned codec SDK with source/rust/Setup-Dependencies.ps1 first.' }
$codecBuild = Get-Content -LiteralPath $codecStamp -Raw | ConvertFrom-Json
foreach ($dll in $codecBuild.dlls.PSObject.Properties) {
    $path = Join-Path $root $dll.Name
    if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $dll.Value) {
        throw "Deployed $($dll.Name) is not from the recorded source build. Run source/build.ps1."
    }
    Assert-NoLocalIdentity $path
}
# Unique staging avoids deleting a previous release or unrelated files.
$dist = Join-Path $root 'dist'
$stage = Join-Path $dist ("draft-" + [guid]::NewGuid().ToString('N'))
$source = Join-Path $stage "AirBridge-$version-source"
$runtime = Join-Path $stage "AirBridge-$version-windows-x64"
New-Item -ItemType Directory -Force $source,$runtime | Out-Null

function Copy-SourceFile([IO.FileInfo]$file) {
    $relative = [IO.Path]::GetRelativePath($root,$file.FullName)
    if ($relative -match '(^|[\\/])(__pycache__|target|vendor|\.git)([\\/]|$)' -or
        $file.Name -match '\.pyc$|identity\.key$|^\.cargo-ok$|^build-manifest\.sha256$') { return }
    $destination = Join-Path $source $relative
    Assert-NoLocalIdentity $file.FullName
    New-Item -ItemType Directory -Force (Split-Path $destination) | Out-Null
    Copy-Item -LiteralPath $file.FullName -Destination $destination
}

foreach ($name in @('README.md','LICENSE','THIRD_PARTY_NOTICES_RUST.md','CONTRIBUTING.md','.gitignore','.gitattributes','source/build.ps1')) {
    Copy-SourceFile (Get-Item -LiteralPath (Join-Path $root $name))
}
foreach ($directory in @('assets','docs','scripts','.github','source/rust/src','source/rust/locales','source/rust/crates','source/rust/tests','source/rust/fixtures','source/rust/licenses')) {
    Get-ChildItem -LiteralPath (Join-Path $root $directory) -File -Recurse | ForEach-Object { Copy-SourceFile $_ }
}
Get-ChildItem -LiteralPath "$root/source/rust" -File | ForEach-Object { Copy-SourceFile $_ }
# Include the exact Rust dependency sources with the source release, not the runtime.
$rustVendor = Join-Path $root 'source/rust/tools/release-rust-sources'
Push-Location "$root/source/rust"
try {
    & cargo vendor --locked --versioned-dirs $rustVendor 1> "$root/source/rust/tools/cargo-vendor-config.txt" 2> "$root/source/rust/tools/cargo-vendor.log"
    if ($LASTEXITCODE -ne 0) { throw 'Could not prepare corresponding Rust dependency sources' }
} finally { Pop-Location }
New-Item -ItemType Directory -Force "$source/source/rust/vendor","$source/source/rust/.cargo" | Out-Null
Copy-Item -LiteralPath $rustVendor -Destination "$source/source/rust/vendor/cargo" -Recurse
$cargoConfig = @'
[source.crates-io]
replace-with = "vendored-sources"
[source.vendored-sources]
directory = "vendor/cargo"
'@
[IO.File]::WriteAllText("$source/source/rust/.cargo/config.toml",$cargoConfig)
$sourceArchive = Join-Path $dist "AirBridge-$version-source.zip"
# Some published Cargo archives use pre-1980 timestamps, outside ZIP's DOS range.
# Normalize only the snapshot metadata; dependency contents/checksums stay intact.
Get-ChildItem -LiteralPath $source -File -Recurse | Where-Object { $_.LastWriteTime.Year -lt 1980 } |
    ForEach-Object { $_.LastWriteTime = [datetime]'1980-01-01' }
Compress-Archive -LiteralPath $source -DestinationPath $sourceArchive -Force

$codecSource = Join-Path $stage "AirBridge-$version-codec-sources"
New-Item -ItemType Directory -Force "$codecSource/sources" | Out-Null
$pins = Get-Content -LiteralPath "$root/source/rust/codec-sources.json" -Raw | ConvertFrom-Json
foreach ($name in @('ffmpeg','x264','zlib')) {
    $archive = Join-Path $codecCache "$name.tar.gz"
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $pins.$name.sha256) { throw "Wrong $name source archive" }
    Copy-Item -LiteralPath $archive -Destination "$codecSource/sources"
}
foreach ($name in @('Build-FFmpeg.ps1','build-codecs.sh','codec-sources.json')) {
    Copy-Item -LiteralPath "$root/source/rust/$name" -Destination $codecSource
}
Copy-Item -LiteralPath "$codecSdk/BUILD-CONFIG.mak","$codecSdk/AIRBRIDGE-CODECS.json" -Destination $codecSource
Copy-Item -LiteralPath "$root/source/rust/FFMPEG-LICENSE.txt","$root/source/rust/licenses/X264-COPYING.txt","$root/source/rust/licenses/ZLIB-README.txt" -Destination $codecSource
$codecReadme = @'
# Corresponding source for AirBridge codec DLLs

The unmodified FFmpeg, x264 and zlib source archives are in sources/. Exact revisions,
URLs and SHA-256 checksums are in codec-sources.json. BUILD-CONFIG.mak records FFmpeg
configuration; AIRBRIDGE-CODECS.json records the shipped DLL hashes and compiler version.
The build recipe is Build-FFmpeg.ps1 plus build-codecs.sh. Original archives are
unmodified. The recipe removes the POSIX unistd.h include from the installed zconf.h
for MSVC compatibility, as recommended by FFmpeg's Windows build guide.
FFmpeg is configured GPLv3-or-later; x264 is GPLv2-or-later; zlib uses its
own permissive license. Original license texts are retained in the source archives.

To rebuild, install Visual Studio C++ tools/Windows SDK, Git for Windows and PowerShell 7.
Extract this directory, copy sources/*.tar.gz into tools/codec-build/, then run:

    pwsh -File ./Build-FFmpeg.ps1

The helper checks hashes, downloads the pinned build tools (make, NASM, pkgconf),
and builds the shared libraries into vendor/ffmpeg-airbridge-9.0.2. Build tools and
Windows system libraries are prerequisites, not additional linked codec libraries.
Only x264 and zlib are statically incorporated into the FFmpeg DLLs in this configuration.
'@
[IO.File]::WriteAllText("$codecSource/README.md",$codecReadme)
Get-ChildItem -LiteralPath $codecSource -File | ForEach-Object { Assert-NoLocalIdentity $_.FullName }
$codecArchive = Join-Path $dist "AirBridge-$version-codec-sources.zip"
Compress-Archive -LiteralPath $codecSource -DestinationPath $codecArchive -Force

$runtimeFiles = @('AirBridge.exe','avcodec-63.dll','avformat-63.dll','avutil-61.dll','swresample-7.dll','swscale-10.dll')
foreach ($name in $runtimeFiles) {
    Copy-Item -LiteralPath (Join-Path $root $name) -Destination $runtime
}
$binaryVersion = (Get-Item -LiteralPath "$runtime/AirBridge.exe").VersionInfo.ProductVersion
if ($binaryVersion -ne $version) { throw "EXE version $binaryVersion does not match source $version; rebuild before packaging" }
Copy-Item -LiteralPath "$root/LICENSE","$root/THIRD_PARTY_NOTICES_RUST.md" -Destination $runtime
$instructions = @"
# AirBridge $version — Windows x64 preview

Keep AirBridge.exe and all five DLLs in this folder. Run AirBridge.exe,
connect the PC and iPhone/iPad to the same LAN, then choose
Control Center → Screen Mirroring → AirBridge on the phone.
F11: fullscreen. Esc: leave fullscreen. R: rotate.
If discovery is blocked, check guest Wi-Fi, VPN and firewall settings.
The in-app local-network action requests administrator approval.

## Русский

Запустите AirBridge.exe; пять DLL должны лежать рядом. Подключите ПК
и iPhone/iPad к одной сети. На телефоне: Пункт управления → Повтор экрана
→ AirBridge. F11 — полный экран, Esc — выход, R — поворот.
При проблемах проверьте VPN, гостевой Wi-Fi и брандмауэр.

This is a preview. DRM and direct HLS playback are unsupported.
Source: AirBridge-$version-source.zip. See LICENSE, licenses/ and
THIRD_PARTY_NOTICES_RUST.md for licensing and dependency source information.
The matching FFmpeg/x264/zlib source and recipes are supplied separately as
AirBridge-$version-codec-sources.zip alongside the application release.
"@
[IO.File]::WriteAllText("$runtime/README.md",$instructions)
New-Item -ItemType Directory -Force "$runtime/licenses" | Out-Null
Copy-Item -LiteralPath "$root/assets/fonts/LICENSE-Inter.txt","$root/source/rust/FFMPEG-LICENSE.txt","$root/source/rust/FFMPEG-BUILD.txt","$root/source/rust/crates/rust-airplay/THIRD_PARTY_NOTICES.md" -Destination "$runtime/licenses"
Copy-Item -LiteralPath "$root/source/rust/licenses/X264-COPYING.txt","$root/source/rust/licenses/ZLIB-README.txt" -Destination "$runtime/licenses"
Copy-Item -LiteralPath $codecStamp -Destination "$runtime/licenses"
$runtimeHashes = Get-ChildItem -LiteralPath $runtime -File -Recurse | Sort-Object FullName | ForEach-Object {
    $relative = [IO.Path]::GetRelativePath($runtime,$_.FullName).Replace('\','/')
    "{0}  {1}" -f (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant(),$relative
}
[IO.File]::WriteAllLines("$runtime/SHA256SUMS.txt",[string[]]$runtimeHashes)
$portable = Join-Path $dist "AirBridge-$version-windows-x64-portable.zip"
Compress-Archive -LiteralPath $runtime -DestinationPath $portable -Force
$artifacts = @($sourceArchive,$codecArchive,$portable)

if ($Installer) {
    if (-not $IsccPath) {
        $compiler = Get-Command ISCC.exe -ErrorAction SilentlyContinue
        if ($compiler) { $IsccPath = $compiler.Source }
        else { $IsccPath = "${env:ProgramFiles(x86)}/Inno Setup 6/ISCC.exe" }
    }
    if (-not (Test-Path -LiteralPath $IsccPath)) { throw 'Inno Setup compiler not found. Supply -IsccPath. Portable/source ZIPs have been created.' }
    & $IsccPath "/DAppVersion=$version" "/DPayloadDir=$runtime" "/DReleaseDir=$dist" "$PSScriptRoot/AirBridge.iss"
    if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed' }
    $artifacts += Join-Path $dist "AirBridge-$version-windows-x64-setup.exe"
}
$hashes = foreach ($artifact in $artifacts) {
    "{0}  {1}" -f (Get-FileHash -LiteralPath $artifact -Algorithm SHA256).Hash.ToLowerInvariant(),[IO.Path]::GetFileName($artifact)
}
[IO.File]::WriteAllLines((Join-Path $dist 'SHA256SUMS.txt'),[string[]]$hashes)
Write-Output $artifacts
Write-Output 'Matching application/dependency source, codec source and binary packages prepared. Validate device playback and clean-account installation before publishing.'
