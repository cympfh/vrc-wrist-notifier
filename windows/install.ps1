# SPDX-License-Identifier: MIT
# vrc-wrist-notifier (Windows side) installer. User space only, no admin, no autostart.
#   irm https://raw.githubusercontent.com/cympfh/vrc-wrist-notifier/main/windows/install.ps1 | iex
# Uninstall (moves files aside; config is kept):
#   irm https://raw.githubusercontent.com/cympfh/vrc-wrist-notifier/main/windows/uninstall.ps1 | iex
param([switch]$Uninstall)
$ErrorActionPreference = "Stop"
$repo = "cympfh/vrc-wrist-notifier"
$ref = if ($env:VRC_WRIST_REF) { $env:VRC_WRIST_REF } else { "main" }
$dest = Join-Path $env:LOCALAPPDATA "vrc-wrist-notifier"
$confDir = Join-Path $env:APPDATA "vrc-wrist-notifier"
$stamp = Get-Date -Format "yyyyMMdd-HHmmss"

if ($Uninstall) {
    if (Test-Path -LiteralPath $dest) {
        $aside = Join-Path $env:LOCALAPPDATA "vrc-wrist-notifier-uninstalled\$stamp"
        New-Item -ItemType Directory -Force -Path (Split-Path $aside) | Out-Null
        Move-Item -LiteralPath $dest -Destination $aside
        Write-Host "Moved $dest to $aside (config in $confDir is kept)."
    } else {
        Write-Host "Not installed."
    }
    return
}

New-Item -ItemType Directory -Force -Path $dest, $confDir | Out-Null
$wc = New-Object System.Net.WebClient
foreach ($f in @("vrc-wrist-notifier.ps1", "config.example.json")) {
    $url = "https://raw.githubusercontent.com/$repo/$ref/windows/$f"
    $tmp = Join-Path $dest "$f.new"
    $wc.DownloadFile($url, $tmp)   # bytes as-is (keeps the UTF-8 BOM)
    Move-Item -Force -LiteralPath $tmp -Destination (Join-Path $dest $f)
}
$conf = Join-Path $confDir "config.json"
if (-not (Test-Path -LiteralPath $conf)) {
    Copy-Item -LiteralPath (Join-Path $dest "config.example.json") -Destination $conf
    Write-Host "Created config from the sample: $conf"
} else {
    Write-Host "Kept existing config: $conf"
}
$run = "powershell -ExecutionPolicy Bypass -File `"$dest\vrc-wrist-notifier.ps1`""
Write-Host ""
Write-Host "Installed to $dest"
Write-Host "Start : $run"
Write-Host "Test  : $run -Test"
Write-Host "Stop  : Ctrl+C in that window"
