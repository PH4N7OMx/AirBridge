#requires -Version 7.0
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'Build-FFmpeg.ps1')
$tools = Join-Path $PSScriptRoot 'tools'
$clang = Join-Path $tools 'clang/clang/native/libclang.dll'
if (-not (Test-Path -LiteralPath $clang)) {
    & py -3 -m pip install --target (Join-Path $tools 'clang') 'libclang==18.1.1'
    if ($LASTEXITCODE -ne 0) { throw 'Could not install local libclang' }
}
'Dependencies ready. Run source/build.ps1 from the project root.'
