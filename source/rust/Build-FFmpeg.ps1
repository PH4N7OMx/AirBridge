#requires -Version 7.0
param([int]$Jobs = 8, [switch]$Force)
$ErrorActionPreference = 'Stop'
$cache = Join-Path $PSScriptRoot 'tools/codec-build'
$sdk = Join-Path $PSScriptRoot 'vendor/ffmpeg-airbridge-9.0.2'
$recipeFiles = @('Build-FFmpeg.ps1','build-codecs.sh','codec-sources.json')
$recipe = ($recipeFiles | ForEach-Object { (Get-FileHash -LiteralPath (Join-Path $PSScriptRoot $_) -Algorithm SHA256).Hash }) -join ':'
$stamp = Join-Path $sdk 'AIRBRIDGE-CODECS.json'
if (-not $Force -and (Test-Path -LiteralPath $stamp)) {
    $previous = Get-Content -LiteralPath $stamp -Raw | ConvertFrom-Json
    if ($previous.recipe -eq $recipe) {
        $valid = $true
        foreach ($dll in $previous.dlls.PSObject.Properties) {
            $path = Join-Path "$sdk/bin" $dll.Name
            if (-not (Test-Path -LiteralPath $path) -or
                (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $dll.Value) { $valid = $false; break }
        }
        if ($valid) { 'Verified existing source-built FFmpeg SDK.'; return }
    }
}
$manifest = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'codec-sources.json') -Raw | ConvertFrom-Json
New-Item -ItemType Directory -Force $cache | Out-Null
foreach ($name in @('ffmpeg','x264','zlib','make','nasm','pkgconf')) {
    $entry = $manifest.$name
    $suffix = if ($name -in @('make','pkgconf')) { 'pkg.tar.zst' } elseif ($name -eq 'nasm') { 'zip' } else { 'tar.gz' }
    $archive = Join-Path $cache "$name.$suffix"
    if (-not (Test-Path -LiteralPath $archive)) { Invoke-WebRequest -Uri $entry.url -OutFile $archive }
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $entry.sha256) { throw "$name checksum mismatch" }
    $folder = Join-Path $cache $name
    if (-not (Test-Path -LiteralPath $folder)) {
        New-Item -ItemType Directory -Force $folder | Out-Null
        if ($name -eq 'nasm') { Expand-Archive -LiteralPath $archive -DestinationPath $folder }
        elseif ($name -in @('make','pkgconf')) { & tar -xf $archive -C $folder }
        else { & tar -xf $archive -C $folder --strip-components=1 }
        if ($LASTEXITCODE -ne 0) { throw "Cannot extract $name" }
    }
}
$vswhere = "${env:ProgramFiles(x86)}/Microsoft Visual Studio/Installer/vswhere.exe"
$vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vs) { throw 'Visual Studio C++ tools are required' }
$developerCommand = Join-Path $vs 'Common7/Tools/VsDevCmd.bat'
$oldEnvironment = @{}
Get-ChildItem Env: | ForEach-Object { $oldEnvironment[$_.Name] = $_.Value }
try {
    # Consume the environment internally; never print credentials from `set`.
    $developerEnvironment = & cmd.exe /d /c "call `"$developerCommand`" -arch=x64 -host_arch=x64 >nul && set"
    if ($LASTEXITCODE -ne 0) { throw 'Cannot initialize MSVC environment' }
    foreach ($line in $developerEnvironment) {
        if ($line -match '^([^=]+)=(.*)$') { Set-Item -LiteralPath "Env:$($Matches[1])" -Value $Matches[2] }
    }
    $git = Split-Path (Split-Path (Get-Command git.exe).Source)
    $bash = Join-Path $git 'bin/bash.exe'
    $nasm = (Get-ChildItem -LiteralPath "$cache/nasm" -Filter nasm.exe -Recurse | Select-Object -First 1).DirectoryName
    $env:PATH = "$cache/make/usr/bin;$cache/pkgconf/usr/bin;$nasm;$env:PATH;$git/usr/bin"
    # Git's `link` is not Microsoft's linker. Preserve the MSVC PATH order.
    $env:MSYS2_PATH_TYPE = 'inherit'
    $env:MSYS_NO_PATHCONV = '1'
    # Header diagnostics can otherwise embed absolute __FILE__ paths in DLLs.
    # MSVC requires deterministic mode for /pathmap to take effect.
    $env:CL = "$env:CL /experimental:deterministic /pathmap:`"$cache=/source/codecs`""
    & $bash (Join-Path $PSScriptRoot 'build-codecs.sh') $cache $sdk $Jobs
    if ($LASTEXITCODE -ne 0) { throw 'Codec build failed' }
    # FFmpeg's MSVC install puts import libraries beside DLLs; Cargo expects lib/.
    Get-ChildItem -LiteralPath "$sdk/bin" -Filter '*.lib' | Copy-Item -Destination "$sdk/lib"
    Copy-Item -LiteralPath "$cache/ffmpeg/COPYING.GPLv3" -Destination "$sdk/LICENSE"
    Copy-Item -LiteralPath "$cache/ffmpeg/ffbuild/config.mak" -Destination "$sdk/BUILD-CONFIG.mak"
    $dlls = @{}
    foreach ($name in @('avcodec-63.dll','avformat-63.dll','avutil-61.dll','swresample-7.dll','swscale-10.dll')) {
        $dlls[$name] = (Get-FileHash -LiteralPath "$sdk/bin/$name" -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    @{recipe=$recipe;ffmpeg_version='9.0.2';msvc_version=$env:VCToolsVersion;dlls=$dlls} |
        ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $stamp
    'FFmpeg codec SDK built from the pinned sources.'
} finally {
    foreach ($item in Get-ChildItem Env:) {
        if (-not $oldEnvironment.ContainsKey($item.Name)) { Remove-Item -LiteralPath "Env:$($item.Name)" }
    }
    foreach ($name in $oldEnvironment.Keys) { Set-Item -LiteralPath "Env:$name" -Value $oldEnvironment[$name] }
}
