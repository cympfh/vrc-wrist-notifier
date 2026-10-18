# 開発者向けメモ

## しくみ

```
VRChat ──書く──▶ output_log_*.txt ──読む──▶ vrc-wrist-notifier.ps1 (Windows)
                                                  │ UDP / OSC  /wrist/notify ,sss  kind title text
                                                  ▼
                                    wrist-toast (Steam Frame, port 39570) ──▶ SteamVR オーバーレイ
```

- Windows 側: PowerShell 5.1 だけで動きます。最新の `output_log_*.txt` を 0.5 秒ごとに読み足し（共有モードで開くので VRChat の邪魔をしません）、新しいログファイルができたら乗り換えます。起動前の行は通知しません
- Frame 側: C++。cairo + pango（Noto Sans CJK JP）で 640×160 のカードを描き、OpenVR の `SetOverlayRaw` で表示します。`libopenvr_api.so` は SteamVR に入っているものを使います（RUNPATH `/opt/steamvr/bin/linuxarm64`）
- SteamVR が動いていないあいだは 5 秒ごとにつなぎ直し、届いた通知は捨てます

## 履歴とダッシュボードのパネル（0.2.0）

- Windows 側は、参加・退出・ワールドの通知に 4〜7 個目の文字列を付けます: 時刻（`yyyy-MM-dd HH:mm:ss`、VRChat のログの時刻＝PC の時刻）、ワールド名、重複を除く鍵（`時刻|種類|名前`）、`quiet`（通知は出さず履歴だけ。ワールドに入った直後の参加など）。時刻の無い通知（`-Test`、0.1.0 の Windows 側）は履歴に残しません
- 起動時（`backfill_history`）と `-Backfill` で、今ある `output_log_*.txt` から参加・退出・ワールドを `/wrist/history ,sssss 時刻 種類 名前 ワールド 鍵` で送ります（通知は出ない）。同じ鍵は Frame 側で 1 回だけ記録します
- Frame 側は `~/.local/state/vrc-wrist-notifier/history.jsonl` に 1 行 1 件で追記します（`{"t","k","n","w","key"}`）。2 MB を超えたら `history.1.jsonl` に回します（1 世代だけ残すので、合計でおよそ 4 MB まで）。このファイルはリポジトリには入りません
- パネルは `CreateDashboardOverlay`（キー `cympfh.vrc-wrist-notifier.history`、1200×800 px、幅 2.6 m、マウス入力、`SendVRSmoothScrollEvents`）。wrist-toast が SteamVR につないだときにその場で登録するので、SteamVR の再起動は要りません。wrist-toast が止まるとパネルも消えます
- 描き直すのはパネルが開いているときだけ。5 秒ごとに `FindOverlay` で自分のパネルが残っているか確かめ、消えていたら作り直します
- 画像は Vulkan のテクスチャ（`SetOverlayTexture`）で渡します。部品は [frame-mic-tuner](https://github.com/sasaken1102r/frame-mic-tuner) の `vk_texture.*`（MIT、`third_party/frame-mic-tuner/`）。`SetOverlayRaw` はちらつきとコンポジタの不調の疑いがあると同プロジェクトに記録があるため、Vulkan が使えないときだけの予備です
- `wrist-toast --probe` で、動いている wrist-toast のパネルが SteamVR にあるかを別のプロセスから確かめられます。`--dump-history-png out.png [--history-dir D] [--filter join|leave] [--scroll PX]` で、SteamVR なしでパネルを PNG にできます

## 拾うログの行

VRChat のログ出力を Full にすると出る行です（形式は [VRCX](https://github.com/vrcx-team/VRCX) の LogWatcher と同じ）。

| 種類 | 行（`YYYY.MM.DD HH:MM:SS Log - ` のあと） |
|---|---|
| 参加 | `[Behaviour] OnPlayerJoined 名前 (usr_…)` |
| 退出 | `[Behaviour] OnPlayerLeft 名前 (usr_…)` |
| ワールド | `[Behaviour] Entering Room: ワールド名` |
| 招待など | `[API] Received Notification: <Notification from username:名前, sender user id:usr_… to … of type: invite, id: not_…, …>` |

通知の種類は `invite` / `requestInvite` / `friendRequest` を出し、それ以外は `notify_other` で切り替えます。同じ `not_…` は 1 回だけ出します。

## 設定

Frame `~/.config/vrc-wrist-notifier/config.ini`

| 項目 | 既定 | 意味 |
|---|---|---|
| listen_port | 39570 | 待ち受ける UDP ポート |
| listen_address | 0.0.0.0 | 待ち受けるアドレス |
| allow_from | lan | 受け付ける送り元（`lan` / `any` / IP のカンマ区切り） |
| anchor | view | `view` / `left_wrist` / `right_wrist`（コントローラーが無ければ view） |
| duration_s | 4 | 1 件の表示秒数（前後 0.4 秒でフェード） |
| width_m | 0.30 | カードの幅（メートル） |
| distance_m / offset_y_m | 0.8 / -0.18 | view のときの距離と高さ |
| max_queue | 8 | 表示待ちの最大件数 |
| font | Noto Sans CJK JP | フォント |
| history_panel | true | ダッシュボードに履歴のパネルを出す |
| panel_width_m | 2.6 | パネルの幅（メートル） |
| history_dir | （空） | 履歴の場所。空なら `~/.local/state/vrc-wrist-notifier` |

Windows `%APPDATA%\vrc-wrist-notifier\config.json`: `frame_host`, `frame_port`, `log_dir`（空なら既定の場所）, `quiet_seconds`, `notify_join` / `notify_leave` / `notify_world` / `notify_invite` / `notify_friend` / `notify_other`、`backfill_history`（起動時に過去のログから履歴を送る、既定 true）。

## テスト

```sh
frame/build.sh /tmp/wrist-toast && /tmp/wrist-toast --self-test       # Frame 上で（g++・cairo・pango は SteamOS に入っている）
/tmp/wrist-toast --dump-png /tmp/t.png --kind invite --title 名前 --text 本文   # SteamVR なしで見た目を確認
```

```powershell
powershell -ExecutionPolicy Bypass -File tests\run-windows-tests.ps1   # 合成ログで検出結果を確認
powershell -ExecutionPolicy Bypass -File windows\vrc-wrist-notifier.ps1 -Replay tests\sample_output_log.txt -DryRun
```

`tests/sample_output_log.txt` と `tests/sample_history_log.txt` は形式だけまねた合成データです（実在の人やワールドは入っていません）。

## 自分で OSC を送る

`/wrist/notify` に文字列 3 つ（種類、タイトル、本文）を送れば、VRChat 以外からでも表示できます。種類は `join` / `leave` / `world` / `invite` / `requestInvite` / `friendRequest` / それ以外（青）。文字列 1 つなら本文だけ、2 つならタイトルと本文として扱います。

## リリース

Frame 上で `frame/build.sh` したものを `wrist-toast-linux-arm64` として GitHub Releases に置き、`SHA256SUMS` を添えます。インストーラーは sha256 と `--self-test` を確かめてから使い、だめなときはその場でビルドします。

## ライセンス

MIT。`vendor/openvr/openvr.h` は Valve の OpenVR SDK（BSD 3-Clause、`vendor/openvr/LICENSE`）。`third_party/frame-mic-tuner/vk_texture.*` は sasaken1102r さんの frame-mic-tuner から（MIT、`third_party/frame-mic-tuner/LICENSE`）。
