# SPDX-License-Identifier: MIT
# vrc-wrist-notifier のインストーラー。管理者の権限は要りません。
#   irm https://raw.githubusercontent.com/cympfh/vrc-wrist-notifier/main/windows/install.ps1 | iex
# やること: アプリ (vrc-wrist-notifier.exe) を %LOCALAPPDATA%\vrc-wrist-notifier に置き、サインインしたときに
# 自動で起動するようにして（スタートアップにショートカット）、今すぐ起動します。アプリはシステムトレイに入り、
# SteamVR が動いているときだけ、そこに通知と「入退室の履歴」を出します。
# 0.4 までの PowerShell 版が動いていれば止めて、置き換えます。
# アンインストール（アプリを止めて自動起動をやめ、ファイルと履歴は脇に移します。設定は残します）:
#   irm https://raw.githubusercontent.com/cympfh/vrc-wrist-notifier/main/windows/uninstall.ps1 | iex
param([switch]$Uninstall)
$ErrorActionPreference = "Stop"
$repo = "cympfh/vrc-wrist-notifier"
$exeName = "vrc-wrist-notifier.exe"
$dest = Join-Path $env:LOCALAPPDATA "vrc-wrist-notifier"
$confDir = Join-Path $env:APPDATA "vrc-wrist-notifier"
$startup = [Environment]::GetFolderPath("Startup")
$lnk = Join-Path $startup "vrc-wrist-notifier.lnk"
$oldCmd = Join-Path $startup "vrc-wrist-notifier.cmd"   # 0.4 までの自動起動
$stamp = Get-Date -Format "yyyyMMdd-HHmmss"

function Stop-Notifier {
    # アプリ（0.5 から）と、0.4 までの PowerShell 版の見張り
    Get-Process -Name "vrc-wrist-notifier" -ErrorAction SilentlyContinue | Stop-Process -Force
    Get-CimInstance Win32_Process -Filter "Name='powershell.exe' OR Name='pwsh.exe'" |
        Where-Object { $_.CommandLine -like "*vrc-wrist-notifier.ps1*" } |
        ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 500
}

if ($Uninstall) {
    Stop-Notifier
    foreach ($f in @($lnk, $oldCmd)) { if (Test-Path -LiteralPath $f) { Remove-Item -LiteralPath $f } }
    if (Test-Path -LiteralPath $dest) {
        $aside = Join-Path $env:LOCALAPPDATA "vrc-wrist-notifier-uninstalled\$stamp"
        New-Item -ItemType Directory -Force -Path (Split-Path $aside) | Out-Null
        Move-Item -LiteralPath $dest -Destination $aside
        Write-Host "アプリを止めて、自動起動をやめました。ファイルは $aside に移しました（設定 $confDir は残しています）。"
    } else {
        Write-Host "インストールされていません。"
    }
    return
}

New-Item -ItemType Directory -Force -Path $dest, $confDir | Out-Null
$exe = Join-Path $dest $exeName
$tmp = "$exe.new"
# 置くアプリ: VRC_WRIST_EXE（手元でビルドした exe）> VRC_WRIST_VERSION（その版のリリース）> 最新のリリース
if ($env:VRC_WRIST_EXE) {
    Copy-Item -LiteralPath $env:VRC_WRIST_EXE -Destination $tmp -Force
} else {
    $base = if ($env:VRC_WRIST_VERSION) { "https://github.com/$repo/releases/download/v$($env:VRC_WRIST_VERSION)" }
            else { "https://github.com/$repo/releases/latest/download" }
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    $wc = New-Object System.Net.WebClient
    Write-Host "ダウンロード: $base/$exeName"
    $wc.DownloadFile("$base/$exeName", $tmp)
    # リリースに SHA256SUMS があれば確かめる
    try { $sums = $wc.DownloadString("$base/SHA256SUMS") } catch { $sums = "" }
    $line = ($sums -split "`n") | Where-Object { $_ -match "\s\*?$([regex]::Escape($exeName))\s*$" } | Select-Object -First 1
    if ($line) {
        $want = ($line -split "\s+")[0].ToLower()
        $got = (Get-FileHash -Algorithm SHA256 -LiteralPath $tmp).Hash.ToLower()
        if ($want -ne $got) { Remove-Item -LiteralPath $tmp; throw "ダウンロードした $exeName の SHA256 が合いません" }
    }
}
$sample = Join-Path $dest "config.example.json"
try {
    (New-Object System.Net.WebClient).DownloadFile("https://raw.githubusercontent.com/$repo/main/windows/config.example.json", "$sample.new")
    Move-Item -Force -LiteralPath "$sample.new" -Destination $sample
} catch { Write-Host "設定のサンプルは取れませんでした（なくても動きます）" }

Stop-Notifier
Move-Item -Force -LiteralPath $tmp -Destination $exe
# 0.4 までの PowerShell 版を片付ける（自動起動の .cmd と、スクリプト本体）
foreach ($f in @($oldCmd, (Join-Path $dest "vrc-wrist-notifier.ps1"))) { if (Test-Path -LiteralPath $f) { Remove-Item -LiteralPath $f } }

$conf = Join-Path $confDir "config.json"
if (-not (Test-Path -LiteralPath $conf) -and (Test-Path -LiteralPath $sample)) {
    Copy-Item -LiteralPath $sample -Destination $conf
    Write-Host "設定を作りました: $conf"
}

# サインインしたときに起動する（スタートアップにショートカット）
$sh = New-Object -ComObject WScript.Shell
$s = $sh.CreateShortcut($lnk)
$s.TargetPath = $exe
$s.WorkingDirectory = $dest
$s.Description = "vrc-wrist-notifier（VRChat の参加・退出を SteamVR に通知）"
$s.Save()

Start-Process -FilePath $exe -WorkingDirectory $dest
Write-Host ""
Write-Host "インストールしました: $exe"
Write-Host "自動起動: $lnk"
Write-Host "起動しました。タスクバーの右下（通知領域）のアイコンを右クリックすると、テスト通知と終了ができます。"
