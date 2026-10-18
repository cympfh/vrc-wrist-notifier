# SPDX-License-Identifier: MIT
# vrc-wrist-notifier (Windows 側)
# VRChat のログ (output_log_*.txt) を読むだけで、参加・退出・ワールド移動・招待・フレンド申請を見つけて
# Steam Frame の wrist-toast に UDP (OSC) で送ります。VRChat の設定やファイルは変更しません。
#
#   powershell -ExecutionPolicy Bypass -File vrc-wrist-notifier.ps1            # 見張りを始める（Ctrl+C で終了）
#   powershell -ExecutionPolicy Bypass -File vrc-wrist-notifier.ps1 -Test      # テスト通知を 1 件送る
#   ... -Replay <ログファイル> -DryRun                                         # 送らずに検出結果だけ表示
#   ... -Backfill [-DryRun]                                                    # 過去のログから履歴だけ送って終わる
#   ... -ShowWorld                                                             # いちばん新しいログから、今いるワールドを表示するだけ
#   ... -Status                                                                # VRChat のログ出力が Full になっているかを表示するだけ
[CmdletBinding()]
param(
    [string]$Config = "",
    [switch]$Test,
    [string]$Replay = "",
    [switch]$Backfill,
    [switch]$ShowWorld,
    [switch]$Status,
    [switch]$DryRun
)
$ErrorActionPreference = "Stop"
$Version = "0.3.1"

# ---------------------------------------------------------------- 設定
$defaults = [ordered]@{
    frame_host    = "frame"   # Steam Frame のホスト名か IP
    frame_port    = 39570
    log_dir       = ""        # 空なら %LOCALAPPDATA%Low\VRChat\VRChat
    batch_seconds = 3         # この秒数以内の「参加」「退出」はまとめる（3 人以上なら 1 枚に）
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
# 8 個目はワールドの ID（0.3.0 から。古い Frame 側は 7 個目までしか読まないので、付けても困らない）
function Send-Toast([string]$kind, [string]$title, [string]$text, [string]$ts = "", [string]$world = "", [switch]$quiet, [string]$worldId = "") {
    if ($DryRun) { if (-not $quiet) { Write-Output ("{0}`t{1}`t{2}" -f $kind, $title, $text) }; return }
    $key = ""
    if ($ts) { $key = Get-HistKey $ts $kind $title }
    $q = ""
    if ($quiet) { $q = "quiet" }
    Send-Osc "/wrist/notify" @($kind, $title, $text, $ts, $world, $key, $q, $worldId)
    if ($quiet) { Log "履歴だけ送信: [$kind] $title" } else { Log "送信: [$kind] $title / $text" }
}

# ---------------------------------------------------------------- 参加・退出をまとめる
# 履歴には 1 人ずつ残し（Send-Toast -quiet）、通知カードだけ「種類 + ワールド」ごとにまとめる。
# 最後の 1 人から batch_seconds 秒たったら出す。2 人までは 1 人ずつ、3 人以上は「A さんほか N 人が参加」の 1 枚
$script:pending = [ordered]@{}
function Get-Now([datetime]$logTs) { if ($Replay) { $logTs } else { Get-Date } }
function Add-Pending([string]$kind, [string]$name, [datetime]$logTs) {
    $now = Get-Now $logTs
    $k = "$kind|$($script:world)"
    if (-not $script:pending.Contains($k)) { $script:pending[$k] = @{ kind = $kind; names = New-Object System.Collections.Generic.List[string]; last = $now } }
    $b = $script:pending[$k]; $b.names.Add($name); $b.last = $now
}
function Flush-Pending([datetime]$now, [switch]$all) {
    foreach ($k in @($script:pending.Keys)) {
        $b = $script:pending[$k]
        if (-not $all -and ($now - $b.last).TotalSeconds -lt [double]$cfg.batch_seconds) { continue }
        $script:pending.Remove($k)
        $verb = if ($b.kind -eq "join") { "参加" } else { "退出" }
        if ($b.names.Count -le 2) { foreach ($n in $b.names) { Send-Toast $b.kind $n "が$($verb)しました" } }
        else { Send-Toast $b.kind ("{0} さんほか {1} 人" -f $b.names[0], ($b.names.Count - 1)) "が$verb" }
    }
}

# ---------------------------------------------------------------- ログの解析
# 行の形: "2026.10.18 21:03:15 Log        -  [Behaviour] OnPlayerJoined 名前 (usr_...)"
$reLine   = [regex]'^(\d{4}\.\d\d\.\d\d \d\d:\d\d:\d\d) (\w+)\s+-\s+(.*)$'
$reJoin   = [regex]'^\[Behaviour\] OnPlayerJoined (.+?)(?: \((usr_[0-9a-fA-F-]+)\))?$'
$reLeft   = [regex]'^\[Behaviour\] OnPlayerLeft (.+?)(?: \((usr_[0-9a-fA-F-]+)\))?$'
$reRoom   = [regex]'^\[Behaviour\] Entering Room: (.+)$'
# "Destination set: wrld_xxxx:12345~private(usr_...)~region(jp)" / "Joining wrld_xxxx:..." の wrld_xxxx だけ使う
# （インスタンスの部分には人の ID が入るので捨てる）
$reJoining = [regex]'^\[Behaviour\] (?:Joining|Destination set:) (wrld_[0-9a-fA-F-]+)'
$reNotif  = [regex]'^\[API\] Received Notification: <Notification from username:(.*?), sender user id:(usr_[0-9a-fA-F-]+) to .*? of type: ?(\w+), id: ?(not_[0-9a-fA-F-]+)'
$reWorldN = [regex]'worldName=([^,}]+)'

$script:enteredAt = [datetime]::MinValue
$script:world = ""
$script:worldId = ""
$script:pendingId = ""
$script:seenNotif = @{}
$script:levels = @{}

# ワールドの行なら今のワールドを覚えて "entered"（入った）か "joining"（ID の行）を返す。ほかの行は $null
# 今の VRChat は Destination set（ID）→ Entering Room（名前）→ Joining（ID）の順。古いログは Joining が先
function Update-World([string]$body) {
    $r = $reJoining.Match($body)
    if ($r.Success) {
        # 入った直後の Joining は、今のワールドの ID（Destination set が無かったとき用）
        $id = $r.Groups[1].Value
        if ($script:world -and (-not $script:worldId -or $script:worldId -eq $id) -and -not $script:pendingId -and
            $body.StartsWith("[Behaviour] Joining ")) { $script:worldId = $id }
        else { $script:pendingId = $id }
        return "joining"
    }
    $r = $reRoom.Match($body)
    if ($r.Success) {
        $script:world = $r.Groups[1].Value.Trim()
        $script:worldId = $script:pendingId
        $script:pendingId = ""
        return "entered"
    }
    return $null
}

function Reset-World { $script:world = ""; $script:worldId = ""; $script:pendingId = "" }

function Handle-Line([string]$line) {
    $m = $reLine.Match($line)
    if (-not $m.Success) { return }
    $level = $m.Groups[2].Value
    $script:levels[$level] = 1 + [int]$script:levels[$level]
    $body = $m.Groups[3].Value.TrimEnd()
    $ts = [datetime]::MinValue
    [void][datetime]::TryParseExact($m.Groups[1].Value, "yyyy.MM.dd HH:mm:ss", $null, 0, [ref]$ts)
    $tss = $ts.ToString("yyyy-MM-dd HH:mm:ss")
    if ($Replay) { Flush-Pending $ts }

    $w = Update-World $body
    if ($w -eq "joining") { return }
    if ($w -eq "entered") {
        $script:enteredAt = $ts
        Flush-Pending $ts -all   # 前のワールドの分は先に出す（ワールドが違えば別のカード）
        Send-Toast "world" $script:world "ワールドに入りました" $tss $script:world -quiet:(-not $cfg.notify_world) -worldId $script:worldId
        return
    }
    $r = $reJoin.Match($body)
    if ($r.Success) {
        # ワールドに入った直後は元からいた人なので通知しない（履歴には残す）
        $quiet = ($ts - $script:enteredAt).TotalSeconds -lt [double]$cfg.quiet_seconds
        Send-Toast "join" $r.Groups[1].Value "が参加しました" $tss $script:world -quiet -worldId $script:worldId
        if (-not ($quiet -or -not $cfg.notify_join)) { Add-Pending "join" $r.Groups[1].Value $ts }
        return
    }
    $r = $reLeft.Match($body)
    if ($r.Success) {
        Send-Toast "leave" $r.Groups[1].Value "が退出しました" $tss $script:world -quiet -worldId $script:worldId
        if ($cfg.notify_leave) { Add-Pending "leave" $r.Groups[1].Value $ts }
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

# VRChat のログ出力の設定を、ログの中身から判断する: full / errors（Error しか出ていない）/ unknown（まだ何も出ていない）
# （VRChat はこの設定をログのレベルとして書くので、Log や Debug の行があれば Full）
function Get-LogState {
    $useful = [int]$script:levels["Log"] + [int]$script:levels["Debug"]
    $total = 0
    foreach ($v in $script:levels.Values) { $total += $v }
    if ($useful -gt 0) { "full" } elseif ($total -gt 0) { "errors" } else { "unknown" }
}

# Frame 側に「動いています」を知らせる（30 秒ごと）。古い Frame 側は知らないアドレスとして捨てるだけ
# 引数: 版、ログ出力の状態（full / errors / unknown / nolog）
$script:lastBeat = [datetime]::MinValue
$script:lastState = ""
function Send-Heartbeat([string]$state, [switch]$force) {
    if (-not $force -and $state -eq $script:lastState -and ((Get-Date) - $script:lastBeat).TotalSeconds -lt 30) { return }
    $script:lastBeat = Get-Date; $script:lastState = $state
    try { Send-Osc "/wrist/heartbeat" @($Version, $state) } catch { Log "送信エラー: $($_.Exception.Message)" }
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
    $count = 0
    foreach ($f in $files) {
        Reset-World   # ワールドはログのファイルごと（VRChat を起動しなおすと新しいファイルになる）
        $fs = New-Object System.IO.FileStream($f.FullName, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read,
            ([System.IO.FileShare]::ReadWrite -bor [System.IO.FileShare]::Delete))
        $sr = New-Object System.IO.StreamReader($fs, [System.Text.Encoding]::UTF8)
        try {
            while ($null -ne ($line = $sr.ReadLine())) {
                $m = $reLine.Match($line)
                if (-not $m.Success) { continue }
                $body = $m.Groups[3].Value.TrimEnd()
                $kind = $null; $name = $null
                $w = Update-World $body
                if ($w -eq "joining") { continue }
                if ($w -eq "entered") { $kind = "world"; $name = $script:world }
                else {
                    $r = $reJoin.Match($body)
                    if ($r.Success) { $kind = "join"; $name = $r.Groups[1].Value }
                    else { $r = $reLeft.Match($body); if ($r.Success) { $kind = "leave"; $name = $r.Groups[1].Value } }
                }
                if (-not $kind) { continue }
                $ts = [datetime]::ParseExact($m.Groups[1].Value, "yyyy.MM.dd HH:mm:ss", $null).ToString("yyyy-MM-dd HH:mm:ss")
                if ($DryRun) { Write-Output ("history`t{0}`t{1}`t{2}`t{3}`t{4}" -f $ts, $kind, $name, $script:world, $script:worldId) }
                else {
                    # 6 個目のワールド ID は 0.3.0 から（古い Frame 側は 5 個目までしか読まない）
                    Send-Osc "/wrist/history" @($ts, $kind, $name, $script:world, (Get-HistKey $ts $kind $name), $script:worldId)
                    if ((++$count % 50) -eq 0) { Start-Sleep -Milliseconds 20 }   # 一度に送りすぎない
                }
            }
        } finally { $sr.Dispose(); $fs.Dispose() }
    }
    if (-not $DryRun) { Log "過去のログから履歴を $count 件送りました（$($files.Count) ファイル。重複は Frame 側で除きます）" }
    Reset-World
}

# ログのファイルを頭から読んで、最後に入ったワールドを覚える（見張りを途中から始めたとき用）。
# 速さのため、ワールドの行と通知の行だけ正規表現にかける
function Read-CurrentWorld([System.IO.FileStream]$fs) {
    Reset-World
    [void]$fs.Seek(0, [System.IO.SeekOrigin]::Begin)
    $tmp = New-Object System.IO.StreamReader($fs, [System.Text.Encoding]::UTF8, $false, 65536, $true)
    try {
        while ($null -ne ($l = $tmp.ReadLine())) {
            if ($l.IndexOf("[Behaviour] Entering Room: ", [StringComparison]::Ordinal) -ge 0 -or
                $l.IndexOf("[Behaviour] Joining wrld_", [StringComparison]::Ordinal) -ge 0 -or
                $l.IndexOf("[Behaviour] Destination set: wrld_", [StringComparison]::Ordinal) -ge 0) {
                $mm = $reLine.Match($l)
                if ($mm.Success -and (Update-World $mm.Groups[3].Value.TrimEnd()) -eq "entered") {
                    [void][datetime]::TryParseExact($mm.Groups[1].Value, "yyyy.MM.dd HH:mm:ss", $null, 0, [ref]$script:enteredAt)
                }
            }
        }
    } finally { $tmp.Dispose() }
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

function Get-NewestLog {
    Get-ChildItem -LiteralPath $cfg.log_dir -Filter "output_log_*.txt" -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
}

if ($Status) {
    $newest = Get-NewestLog
    if (-not $newest) { Write-Output "log`tnolog"; exit 0 }
    $fs = New-Object System.IO.FileStream($newest.FullName, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read,
        ([System.IO.FileShare]::ReadWrite -bor [System.IO.FileShare]::Delete))
    try {
        [void]$fs.Seek([Math]::Max(0, $fs.Length - 512KB), [System.IO.SeekOrigin]::Begin)
        $sr = New-Object System.IO.StreamReader($fs, [System.Text.Encoding]::UTF8)
        while ($null -ne ($l = $sr.ReadLine())) {
            $mm = $reLine.Match($l)
            if ($mm.Success) { $script:levels[$mm.Groups[2].Value] = 1 + [int]$script:levels[$mm.Groups[2].Value] }
        }
    } finally { $fs.Dispose() }
    Write-Output ("log`t{0}`t{1}" -f (Get-LogState), $newest.Name)
    exit 0
}

if ($ShowWorld) {
    $newest = Get-NewestLog
    if (-not $newest) { Write-Output "VRChat のログが見つかりません（$($cfg.log_dir)）"; exit 1 }
    $fs = New-Object System.IO.FileStream($newest.FullName, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read,
        ([System.IO.FileShare]::ReadWrite -bor [System.IO.FileShare]::Delete))
    try { Read-CurrentWorld $fs } finally { $fs.Dispose() }
    if ($script:world) { Write-Output ("world`t{0}`t{1}`t{2}" -f $script:world, $script:worldId, $script:enteredAt.ToString("yyyy-MM-dd HH:mm:ss")) }
    else { Write-Output "world`t`t`t" }
    exit 0
}

if ($Replay) {
    foreach ($line in [System.IO.File]::ReadLines((Resolve-Path -LiteralPath $Replay).Path, [System.Text.Encoding]::UTF8)) {
        Handle-Line $line
    }
    Flush-Pending ([datetime]::MaxValue) -all
    if (-not $DryRun) { Warn-IfErrorsOnly }
    exit 0
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
                    # 途中から見はじめたときも、今いるワールドがわかるように
                    Read-CurrentWorld $fs
                    if ($script:world) { Log "今いるワールド: $($script:world)" }
                } else {
                    Reset-World   # 新しいログ = VRChat を起動しなおした
                    $script:levels = @{}
                }
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
        if ($reader) { Send-Heartbeat (Get-LogState) } else { Send-Heartbeat "nolog" }
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
        try { Flush-Pending (Get-Date) } catch { Log "送信エラー: $($_.Exception.Message)" }
        Start-Sleep -Milliseconds 500
    }
} finally {
    if ($reader) { $reader.Dispose() }
    if ($fs) { $fs.Dispose() }
    if ($script:udp) { $script:udp.Dispose() }
}
