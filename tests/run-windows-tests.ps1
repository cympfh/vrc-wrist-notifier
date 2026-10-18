# SPDX-License-Identifier: MIT
# 合成ログ (sample_output_log.txt) を -Replay -DryRun で読み、検出結果が expected_events.tsv と一致するか確かめる。
#   powershell -ExecutionPolicy Bypass -File tests\run-windows-tests.ps1
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$script = Join-Path $here "..\windows\vrc-wrist-notifier.ps1"
$noConfig = Join-Path $env:TEMP "vrc-wrist-notifier-test-no-config.json"
$got = @(& $script -Config $noConfig -Replay (Join-Path $here "sample_output_log.txt") -DryRun)
$want = @(Get-Content -LiteralPath (Join-Path $here "expected_events.tsv") -Encoding UTF8)
$fail = 0
$n = [Math]::Max($got.Count, $want.Count)
for ($i = 0; $i -lt $n; $i++) {
    if ($got[$i] -ceq $want[$i]) { Write-Host "OK   $($want[$i])" }
    else { Write-Host "FAIL want: $($want[$i])"; Write-Host "     got : $($got[$i])"; $fail++ }
}

# 過去のログからの履歴（-Backfill）: 合成ログを一時フォルダに output_log_*.txt として置いて読む
$tmp = Join-Path $env:TEMP ("vrc-wrist-notifier-test-" + [guid]::NewGuid().ToString("N").Substring(0, 8))
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
Copy-Item -LiteralPath (Join-Path $here "sample_output_log.txt") -Destination (Join-Path $tmp "output_log_2026-01-02_21-00-00.txt")
# 2 つ目のファイル: ワールドの行より前の参加（前のファイルのワールドを引き継がない）、絵文字のワールド名、ワールド ID
Copy-Item -LiteralPath (Join-Path $here "sample_world_log.txt") -Destination (Join-Path $tmp "output_log_2026-01-03_20-00-00.txt")
(Get-Item (Join-Path $tmp "output_log_2026-01-02_21-00-00.txt")).LastWriteTime = [datetime]"2026-01-02 22:00:00"
(Get-Item (Join-Path $tmp "output_log_2026-01-03_20-00-00.txt")).LastWriteTime = [datetime]"2026-01-03 21:00:00"
$cfgPath = Join-Path $tmp "config.json"
[IO.File]::WriteAllText($cfgPath, (@{ log_dir = $tmp } | ConvertTo-Json), (New-Object System.Text.UTF8Encoding $false))
$got2 = @(& $script -Config $cfgPath -Backfill -DryRun)
$want2 = @(Get-Content -LiteralPath (Join-Path $here "expected_history.tsv") -Encoding UTF8)
$n2 = [Math]::Max($got2.Count, $want2.Count)
for ($i = 0; $i -lt $n2; $i++) {
    if ($got2[$i] -ceq $want2[$i]) { Write-Host "OK   $($want2[$i])" }
    else { Write-Host "FAIL want: $($want2[$i])"; Write-Host "     got : $($got2[$i])"; $fail++ }
}
# 今いるワールド（-ShowWorld）: いちばん新しいログの最後のワールド
$got3 = @(& $script -Config $cfgPath -ShowWorld)
$want3 = @(Get-Content -LiteralPath (Join-Path $here "expected_world.tsv") -Encoding UTF8)
if ($got3[0] -ceq $want3[0]) { Write-Host "OK   $($want3[0])" } else { Write-Host "FAIL want: $($want3[0])"; Write-Host "     got : $($got3[0])"; $fail++ }
# 参加・退出のまとめ（1 人、2 人、3 人、大勢、参加と退出が混ざる、ワールドが違う）
$got4 = @(& $script -Config $noConfig -Replay (Join-Path $here "sample_batch_log.txt") -DryRun)
$want4 = @(Get-Content -LiteralPath (Join-Path $here "expected_batch.tsv") -Encoding UTF8)
$n4 = [Math]::Max($got4.Count, $want4.Count)
for ($i = 0; $i -lt $n4; $i++) {
    if ($got4[$i] -ceq $want4[$i]) { Write-Host "OK   $($want4[$i])" }
    else { Write-Host "FAIL want: $($want4[$i])"; Write-Host "     got : $($got4[$i])"; $fail++ }
}
$n += $n4
# フレンド（0.4.0）: friends.json の人は印が付き、まとめカードの先頭にはフレンドが来る。テストでは API を呼ばない
$ftmp = Join-Path $tmp "friend"
New-Item -ItemType Directory -Force -Path $ftmp | Out-Null
Copy-Item -LiteralPath (Join-Path $here "sample_friend_log.txt") -Destination (Join-Path $ftmp "output_log_2026-01-06_20-00-00.txt")
$fcfg = Join-Path $ftmp "config.json"
[IO.File]::WriteAllText($fcfg, (@{ log_dir = $ftmp; friends_file = (Join-Path $here "sample_friends.json") } | ConvertTo-Json),
    (New-Object System.Text.UTF8Encoding $false))
foreach ($t in @(@("-Replay", (Join-Path $here "sample_friend_log.txt"), "expected_friend.tsv"), @("-Backfill", $null, "expected_friend_history.tsv"))) {
    if ($t[1]) { $got5 = @(& $script -Config $fcfg $t[0] $t[1] -DryRun) } else { $got5 = @(& $script -Config $fcfg -Backfill -DryRun) }
    $want5 = @(Get-Content -LiteralPath (Join-Path $here $t[2]) -Encoding UTF8)
    $n5 = [Math]::Max($got5.Count, $want5.Count)
    for ($i = 0; $i -lt $n5; $i++) {
        if ($got5[$i] -ceq $want5[$i]) { Write-Host "OK   $($want5[$i])" }
        else { Write-Host "FAIL want: $($want5[$i])"; Write-Host "     got : $($got5[$i])"; $fail++ }
    }
    $n += $n5
}
Write-Host "（一時フォルダ $tmp は消していません）"
$n += $n2 + 1
if ($fail) { Write-Host "$fail 件失敗"; exit 1 } else { Write-Host "全 $n 件 OK"; exit 0 }
