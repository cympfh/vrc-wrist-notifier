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
$cfgPath = Join-Path $tmp "config.json"
[IO.File]::WriteAllText($cfgPath, (@{ log_dir = $tmp } | ConvertTo-Json), (New-Object System.Text.UTF8Encoding $false))
$got2 = @(& $script -Config $cfgPath -Backfill -DryRun)
$want2 = @(Get-Content -LiteralPath (Join-Path $here "expected_history.tsv") -Encoding UTF8)
$n2 = [Math]::Max($got2.Count, $want2.Count)
for ($i = 0; $i -lt $n2; $i++) {
    if ($got2[$i] -ceq $want2[$i]) { Write-Host "OK   $($want2[$i])" }
    else { Write-Host "FAIL want: $($want2[$i])"; Write-Host "     got : $($got2[$i])"; $fail++ }
}
Write-Host "（一時フォルダ $tmp は消していません）"
$n += $n2
if ($fail) { Write-Host "$fail 件失敗"; exit 1 } else { Write-Host "全 $n 件 OK"; exit 0 }
