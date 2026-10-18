# SPDX-License-Identifier: MIT
# vrc-wrist-notifier (Windows 側)
# VRChat のログ (output_log_*.txt) を読むだけで、参加・退出・ワールド移動・招待・フレンド申請を見つけて
# Steam Frame の wrist-toast に UDP (OSC) で送ります。VRChat の設定やファイルは変更しません。
#
#   powershell -ExecutionPolicy Bypass -File vrc-wrist-notifier.ps1            # 見張りを始める（Ctrl+C で終了）
#   powershell -ExecutionPolicy Bypass -File vrc-wrist-notifier.ps1 -Test      # テスト通知を 1 件送る
#   ... -Replay <ログファイル> -DryRun                                         # 送らずに検出結果だけ表示
#   ... -Backfill [-DryRun]                                                    # 過去のログから履歴だけ送って終わる
[CmdletBinding()]
param(
    [string]$Config = "",
    [switch]$Test,
    [string]$Replay = "",
    [switch]$Backfill,
    [switch]$DryRun
)
$ErrorActionPreference = "Stop"
$Version = "0.2.0"

# ---------------------------------------------------------------- 設定
$defaults = [ordered]@{
    frame_host    = "frame"   # Steam Frame のホスト名か IP
    frame_port    = 39570
    log_dir       = ""        # 空なら %LOCALAPPDATA%Low\VRChat\VRChat
    quiet_seconds = 10        # ワールドに入った直後の「参加」は、元からいた人なので出さない
    notify_join   = $true
    notify_leave  = $true
    notify_world  = $true
    notify_invite = $true     # invite / requestInvite
    notify_friend = $true     # friendRequest
    notify_other  = $false    # それ以外の通知
    backfill_history = $true  # 起動したとき、今ある VRChat のログから参加・退出の履歴を Frame に送る（重複は Frame 側で除く）
}
if (-not $Config) { $Config = Join-Path $env:APPDATA "vrc-wrist-notifier\config.json" }
$cfg = @{}
foreach ($k in $defaults.Keys) { $cfg[$k] = $defaults[$k] }
if (Test-Path -LiteralPath $Config) {
    $json = Get-Content -LiteralPath $Config -Raw -Encoding UTF8 | ConvertFrom-Json
    foreach ($p in $json.PSObject.Properties) {
        if ($cfg.ContainsKey($p.Name)) { $cfg[$p.Name] = $p.Value }
        elseif ($p.Name -notlike "_*") { Write-Warning "知らない設定項目 $($p.Name) は無視します" }
    }
}
if (-not $cfg.log_dir) { $cfg.log_dir = Join-Path ($env:LOCALAPPDATA + "Low") "VRChat\VRChat" }

function Log([string]$msg) { Write-Host ("{0} {1}" -f (Get-Date -Format "HH:mm:ss"), $msg) }

# ---------------------------------------------------------------- OSC 送信
$script:udp = $null
$script:endpoint = $null
function Get-Endpoint {
    $ip = $null
    if (-not [System.Net.IPAddress]::TryParse([string]$cfg.frame_host, [ref]$ip)) {
        # Frame 側は IPv4 で待っているので、IPv4 のアドレスを選ぶ
        $ip = [System.Net.Dns]::GetHostAddresses([string]$cfg.frame_host) |
            Where-Object { $_.AddressFamily -eq [System.Net.Sockets.AddressFamily]::InterNetwork } |
            Select-Object -First 1
        if (-not $ip) { throw "$($cfg.frame_host) の IPv4 アドレスが見つかりません。config.json の frame_host に IP を書いてください。" }
    }
    New-Object System.Net.IPEndPoint($ip, [int]$cfg.frame_port)
}

function Add-OscString([System.Collections.Generic.List[byte]]$buf, [string]$s) {
    $b = [System.Text.Encoding]::UTF8.GetBytes($s)
    $buf.AddRange($b)
    $pad = 4 - ($b.Length % 4)
    for ($i = 0; $i -lt $pad; $i++) { $buf.Add(0) }
}

function Send-Osc([string]$address, [string[]]$strings) {
    if (-not $script:udp) {
        $script:endpoint = Get-Endpoint
        $script:udp = New-Object System.Net.Sockets.UdpClient([System.Net.Sockets.AddressFamily]::InterNetwork)
        Log "送り先: $($script:endpoint)"
    }
    $buf = New-Object System.Collections.Generic.List[byte]
    Add-OscString $buf $address
    Add-OscString $buf ("," + ("s" * $strings.Count))
    foreach ($x in $strings) { Add-OscString $buf $x }
    $bytes = $buf.ToArray()
    [void]$script:udp.Send($bytes, $bytes.Length, $script:endpoint)
}

# 履歴の重複を除く鍵（Frame 側で同じ鍵は 1 回だけ記録する）
function Get-HistKey([string]$ts, [string]$kind, [string]$name) { "$ts|$kind|$name" }

# 通知を送る。$ts（yyyy-MM-dd HH:mm:ss）と $world を付けると Frame 側の履歴にも残る。$quiet なら履歴だけ
function Send-Toast([string]$kind, [string]$title, [string]$text, [string]$ts = "", [string]$world = "", [switch]$quiet) {
    if ($DryRun) { if (-not $quiet) { Write-Output ("{0}`t{1}`t{2}" -f $kind, $title, $text) }; return }
    $key = ""
    if ($ts) { $key = Get-HistKey $ts $kind $title }
    $q = ""
    if ($quiet) { $q = "quiet" }
    Send-Osc "/wrist/notify" @($kind, $title, $text, $ts, $world, $key, $q)
    if ($quiet) { Log "履歴だけ送信: [$kind] $title" } else { Log "送信: [$kind] $title / $text" }
}

# ---------------------------------------------------------------- ログの解析
# 行の形: "2026.10.18 21:03:15 Log        -  [Behaviour] OnPlayerJoined 名前 (usr_...)"
$reLine   = [regex]'^(\d{4}\.\d\d\.\d\d \d\d:\d\d:\d\d) (\w+)\s+-\s+(.*)$'
$reJoin   = [regex]'^\[Behaviour\] OnPlayerJoined (.+?)(?: \((usr_[0-9a-fA-F-]+)\))?$'
$reLeft   = [regex]'^\[Behaviour\] OnPlayerLeft (.+?)(?: \((usr_[0-9a-fA-F-]+)\))?$'
$reRoom   = [regex]'^\[Behaviour\] Entering Room: (.+)$'
$reNotif  = [regex]'^\[API\] Received Notification: <Notification from username:(.*?), sender user id:(usr_[0-9a-fA-F-]+) to .*? of type: ?(\w+), id: ?(not_[0-9a-fA-F-]+)'
$reWorldN = [regex]'worldName=([^,}]+)'

$script:enteredAt = [datetime]::MinValue
$script:world = ""
$script:seenNotif = @{}
$script:levels = @{}

function Handle-Line([string]$line) {
    $m = $reLine.Match($line)
    if (-not $m.Success) { return }
    $level = $m.Groups[2].Value
    $script:levels[$level] = 1 + [int]$script:levels[$level]
    $body = $m.Groups[3].Value.TrimEnd()
    $ts = [datetime]::MinValue
    [void][datetime]::TryParseExact($m.Groups[1].Value, "yyyy.MM.dd HH:mm:ss", $null, 0, [ref]$ts)
    $tss = $ts.ToString("yyyy-MM-dd HH:mm:ss")

    $r = $reRoom.Match($body)
    if ($r.Success) {
        $script:enteredAt = $ts
        $script:world = $r.Groups[1].Value
        Send-Toast "world" $script:world "ワールドに入りました" $tss $script:world -quiet:(-not $cfg.notify_world)
        return
    }
    $r = $reJoin.Match($body)
    if ($r.Success) {
        # ワールドに入った直後は元からいた人なので通知しない（履歴には残す）
        $quiet = ($ts - $script:enteredAt).TotalSeconds -lt [double]$cfg.quiet_seconds
        Send-Toast "join" $r.Groups[1].Value "が参加しました" $tss $script:world -quiet:($quiet -or -not $cfg.notify_join)
        return
    }
    $r = $reLeft.Match($body)
    if ($r.Success) {
        Send-Toast "leave" $r.Groups[1].Value "が退出しました" $tss $script:world -quiet:(-not $cfg.notify_leave)
        return
    }
    $r = $reNotif.Match($body)
    if ($r.Success) {
        $id = $r.Groups[4].Value
        if ($script:seenNotif.ContainsKey($id)) { return }
        $script:seenNotif[$id] = $true
        $who = $r.Groups[1].Value
        $type = $r.Groups[3].Value
        switch ($type) {
            "invite" {
                $w = $reWorldN.Match($body)
                $text = "から招待が届きました"
                if ($w.Success) { $text = "から招待: " + $w.Groups[1].Value.Trim() }
                if ($cfg.notify_invite) { Send-Toast "invite" $who $text }
            }
            "requestInvite" { if ($cfg.notify_invite) { Send-Toast "requestInvite" $who "から招待のリクエスト" } }
            "friendRequest" { if ($cfg.notify_friend) { Send-Toast "friendRequest" $who "からフレンド申請" } }
            default         { if ($cfg.notify_other) { Send-Toast "info" $who "から通知 ($type)" } }
        }
    }
}

function Warn-IfErrorsOnly {
    $useful = [int]$script:levels["Log"] + [int]$script:levels["Debug"]
    $total = 0
    foreach ($v in $script:levels.Values) { $total += $v }
    if ($total -gt 0 -and $useful -eq 0) {
        Write-Warning ("VRChat のログに Error しか出ていません。参加・退出を見つけるには、VRChat の" +
            " クイックメニュー → 設定 → Debug（デバッグ）のログ出力を「Full」にしてください。")
    }
}

# 今ある VRChat のログ（古い順）から、参加・退出・ワールドを履歴として送る。通知は出さない
function Send-Backfill {
    $files = @(Get-ChildItem -LiteralPath $cfg.log_dir -Filter "output_log_*.txt" -ErrorAction SilentlyContinue | Sort-Object LastWriteTime)
    $count = 0; $world = ""
    foreach ($f in $files) {
        $fs = New-Object System.IO.FileStream($f.FullName, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read,
            ([System.IO.FileShare]::ReadWrite -bor [System.IO.FileShare]::Delete))
        $sr = New-Object System.IO.StreamReader($fs, [System.Text.Encoding]::UTF8)
        try {
            while ($null -ne ($line = $sr.ReadLine())) {
                $m = $reLine.Match($line)
                if (-not $m.Success) { continue }
                $body = $m.Groups[3].Value.TrimEnd()
                $kind = $null; $name = $null
                $r = $reRoom.Match($body)
                if ($r.Success) { $kind = "world"; $name = $r.Groups[1].Value; $world = $name }
                else {
                    $r = $reJoin.Match($body)
                    if ($r.Success) { $kind = "join"; $name = $r.Groups[1].Value }
                    else { $r = $reLeft.Match($body); if ($r.Success) { $kind = "leave"; $name = $r.Groups[1].Value } }
                }
                if (-not $kind) { continue }
                $ts = [datetime]::ParseExact($m.Groups[1].Value, "yyyy.MM.dd HH:mm:ss", $null).ToString("yyyy-MM-dd HH:mm:ss")
                if ($DryRun) { Write-Output ("history`t{0}`t{1}`t{2}`t{3}" -f $ts, $kind, $name, $world) }
                else {
                    Send-Osc "/wrist/history" @($ts, $kind, $name, $world, (Get-HistKey $ts $kind $name))
                    if ((++$count % 50) -eq 0) { Start-Sleep -Milliseconds 20 }   # 一度に送りすぎない
                }
            }
        } finally { $sr.Dispose(); $fs.Dispose() }
    }
    if (-not $DryRun) { Log "過去のログから履歴を $count 件送りました（$($files.Count) ファイル。重複は Frame 側で除きます）" }
}

# ---------------------------------------------------------------- 入口
if ($Test) {
    Send-Toast "join" "テスト" "vrc-wrist-notifier から届きました"
    exit 0
}

if ($Backfill) {
    Send-Backfill
    exit 0
}

if ($Replay) {
    foreach ($line in [System.IO.File]::ReadLines((Resolve-Path -LiteralPath $Replay).Path, [System.Text.Encoding]::UTF8)) {
        Handle-Line $line
    }
    if (-not $DryRun) { Warn-IfErrorsOnly }
    exit 0
}

function Get-NewestLog {
    Get-ChildItem -LiteralPath $cfg.log_dir -Filter "output_log_*.txt" -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
}

Log "vrc-wrist-notifier $Version を開始（ログの場所: $($cfg.log_dir)、Ctrl+C で終了）"
if ($cfg.backfill_history) {
    try { Send-Backfill } catch { Log "履歴の読み込みに失敗しました: $($_.Exception.Message)" }
}
$current = $null; $fs = $null; $reader = $null; $partial = ""; $lastCheck = [datetime]::MinValue; $warned = $false
try {
    while ($true) {
        if (((Get-Date) - $lastCheck).TotalSeconds -ge 5) {
            $lastCheck = Get-Date
            $newest = Get-NewestLog
            if ($newest -and (-not $current -or $newest.FullName -ne $current)) {
                if ($reader) { $reader.Dispose(); $fs.Dispose() }
                $isFirst = -not $current
                $current = $newest.FullName
                $fs = New-Object System.IO.FileStream($current, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read,
                    ([System.IO.FileShare]::ReadWrite -bor [System.IO.FileShare]::Delete))
                if ($isFirst) {
                    # 起動前のことは通知しない。Debug 設定の確認と、通知の二重表示を防ぐため、末尾 512KB だけ読む
                    $start = [Math]::Max(0, $fs.Length - 512KB)
                    [void]$fs.Seek($start, [System.IO.SeekOrigin]::Begin)
                    $tmp = New-Object System.IO.StreamReader($fs, [System.Text.Encoding]::UTF8, $false, 65536, $true)

                    while ($null -ne ($l = $tmp.ReadLine())) {
                        $mm = $reLine.Match($l)
                        if ($mm.Success) {
                            $script:levels[$mm.Groups[2].Value] = 1 + [int]$script:levels[$mm.Groups[2].Value]
                            $b = $mm.Groups[3].Value
                            $rn = $reNotif.Match($b); if ($rn.Success) { $script:seenNotif[$rn.Groups[4].Value] = $true }
                        }
                    }
                    $tmp.Dispose()
                    [void]$fs.Seek(0, [System.IO.SeekOrigin]::End)
                    Warn-IfErrorsOnly; $warned = $true
                }
                $reader = New-Object System.IO.StreamReader($fs, [System.Text.Encoding]::UTF8, $false, 65536, $true)
                $partial = ""
                Log "読んでいるログ: $(Split-Path -Leaf $current)"
            }
        }
        if ($reader) {
            $chunk = $reader.ReadToEnd()
            if ($chunk) {
                $data = $partial + $chunk
                $lines = $data -split "`r?`n"
                $partial = $lines[-1]
                for ($i = 0; $i -lt $lines.Length - 1; $i++) {
                    try { Handle-Line $lines[$i] } catch { Log "送信エラー: $($_.Exception.Message)" }
                }
            }
        } elseif (-not $warned) {
            Log "VRChat のログがまだありません。VRChat を起動すると読み始めます。"
            $warned = $true
        }
        Start-Sleep -Milliseconds 500
    }
} finally {
    if ($reader) { $reader.Dispose() }
    if ($fs) { $fs.Dispose() }
    if ($script:udp) { $script:udp.Dispose() }
}
