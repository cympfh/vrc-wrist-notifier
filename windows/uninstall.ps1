# vrc-wrist-notifier (Windows 側) のアンインストーラー（アプリを止めて自動起動をやめ、ファイルは脇に移します。設定は残します）
# 使い方: irm https://raw.githubusercontent.com/cympfh/vrc-wrist-notifier/main/windows/uninstall.ps1 | iex
$src = Invoke-RestMethod -Uri 'https://raw.githubusercontent.com/cympfh/vrc-wrist-notifier/main/windows/install.ps1'
& ([scriptblock]::Create($src)) -Uninstall
