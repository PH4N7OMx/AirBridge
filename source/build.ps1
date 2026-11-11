$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'rust\build.ps1') -Deploy
