# vrc-wrist-notifier (Windows side) uninstaller (moves files aside; config is kept)
# Usage: irm https://raw.githubusercontent.com/cympfh/vrc-wrist-notifier/main/windows/uninstall.ps1 | iex
$src = Invoke-RestMethod -Uri 'https://raw.githubusercontent.com/cympfh/vrc-wrist-notifier/main/windows/install.ps1'
& ([scriptblock]::Create($src)) -Uninstall
