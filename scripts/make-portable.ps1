# Builds the Windows portable zip from a finished release build.
# Usage: ./scripts/make-portable.ps1 [-Target x86_64-pc-windows-msvc]
param([string]$Target = "x86_64-pc-windows-msvc")
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$release = Join-Path $root "target\$Target\release"
if (-not (Test-Path (Join-Path $release "prism.exe"))) { $release = Join-Path $root "target\release" }
$version = (Get-Content (Join-Path $root "app\src-tauri\tauri.conf.json") | ConvertFrom-Json).version
$stage = Join-Path $root "packaging\out\Prism-$version-portable"
$zip = Join-Path $root "packaging\out\Prism-$version-windows-x64-portable.zip"

Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item (Join-Path $release "prism.exe") $stage
$webview2 = Join-Path $release "WebView2Loader.dll"
if (Test-Path $webview2) {
  Copy-Item $webview2 $stage
}
Copy-Item (Join-Path $root "app\src-tauri\binaries\prism-native-host-$Target.exe") (Join-Path $stage "prism-native-host.exe")
# portable.txt switches the data directory to .\data next to prism.exe
Set-Content -Path (Join-Path $stage "portable.txt") -Value "Prism portable mode: settings and history are stored in the data folder next to prism.exe." -Encoding utf8
$ext = Join-Path $root "extension\build"
if (Test-Path $ext) {
  New-Item -ItemType Directory -Force (Join-Path $stage "extension") | Out-Null
  Copy-Item -Recurse (Join-Path $ext "chromium") (Join-Path $stage "extension\chromium")
  Copy-Item -Recurse (Join-Path $ext "firefox") (Join-Path $stage "extension\firefox")
}
Copy-Item (Join-Path $root "packaging\windows\PORTABLE-README.txt") $stage
Remove-Item -Force $zip -ErrorAction SilentlyContinue
# tar.exe (Windows 10+) writes portable forward-slash paths; Compress-Archive on PS 5.1 does not.
$bsdtar = Join-Path $env:SystemRoot "System32\tar.exe"   # not Git's GNU tar, which can't write zips
& $bsdtar -a -c -f $zip -C $stage .
if ($LASTEXITCODE -ne 0 -or -not (Test-Path $zip)) { throw "tar failed to create $zip" }
Write-Host "portable: $zip"
