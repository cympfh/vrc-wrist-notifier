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
if ($fail) { Write-Host "$fail 件失敗"; exit 1 } else { Write-Host "全 $n 件 OK"; exit 0 }
