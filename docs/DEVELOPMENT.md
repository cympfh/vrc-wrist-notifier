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

## ワールド名と見張りの様子（0.3.0）

- Windows 側は今いるワールドの名前と ID（`wrld_…` だけ。インスタンスの部分は人の ID が入るので送らない）を覚え、`/wrist/notify` の 8 個目、`/wrist/history` の 6 個目に ID を付けます。古い Frame 側は余分な引数を読まないだけです
- ワールドはログのファイルごとに数えなおします（VRChat を起動しなおすと新しいファイル）。見張りを途中から始めたときは、今のログファイルを頭から読んで、最後に入ったワールドを覚えます（`-ShowWorld` で同じ読み方の結果だけを表示）
- 履歴の 1 行に `"wid"` が増えました（無ければ書かない）。`w` と `wid` が無い古い行も読めます。同じ鍵の記録が来て、前の記録にワールドが無かったときは、ワールドを書き足した行を追記します（読み直すと後の行が勝つ）。過去のログの読み込みで、ワールド不明だった記録が埋まります
- パネルはワールドに入ったところで区切り、見出しに「入った時刻 〜 最後の出来事の時刻 ・ 来た人数」を出します。ワールドのわからない記録は、前の滞在から 30 分以上あいていたら「（ワールド不明）」として分けます
- Windows 側は 30 秒ごと（状態が変わったときはすぐ）に `/wrist/heartbeat ,ss 版 ログの状態` を送ります。ログの状態は今のログファイルの中身から判断します: Log / Debug の行があれば `full`、Error / Warning しか無ければ `errors`、まだ何も無ければ `unknown`、VRChat のログが無ければ `nolog`（`-Status` で同じ判断を表示）
- Frame 側は 90 秒以内に heartbeat が来ていれば「見張りが動いている」とし、履歴がまだ無いときのパネルに、見張りとデバッグログ（Full）のそれぞれが済んでいるか、済んでいなければ何をすればよいかを出します。heartbeat が無く時刻付きの通知だけが来るときは 0.3.0 より前の Windows 側とみなします
- `--dump-history-png out.png --history-dir 空のフォルダ --watcher on|old|off --vrc-log full|errors|unknown|nolog` で、その表示を PNG にできます

## フレンドの名前を黄色に（0.4.0）

- Windows 側は VRChat のフレンド一覧（`usr_…` の ID だけ）を `%LOCALAPPDATA%\vrc-wrist-notifier\friends.json`（`fetched_at`、`count`、`ids`）に置きます。キーは書きません
- 取り直すのは `friends_refresh_days`（既定 3）日ごとと、一覧に無い人が参加したとき（`friends_unknown_refresh_minutes`、既定 60 分に 1 回まで）だけ。`GET https://api.vrchat.cloud/api/1/auth/user` を 1 回呼んで `friends` を使います（User-Agent `vrc-wrist-notifier/版 cympfh`）。取り直しは別の runspace で行うので、通知は止まりません
- キー（`auth` クッキーの値）は、Windows の環境変数 `VRCHAT_AUTH_KEY`（プロセス → ユーザー）、無ければ `wsl.exe -d <wsl_distro> --exec zsh -lic` で読みます。取り直すときだけ読み、表示・ログ・保存はしません。キーが無い・401 なら、起動しなおすまで自分からは取り直さず、今までどおり動きます
- `/wrist/notify` の 9 個目、`/wrist/history` の 7 個目が `"1"` ならフレンド。まとめカードでは先頭にフレンドを出し、「名前 さんほか N 人」の名前の部分だけ黄色にします
- 一覧に無い人を取り直したあとでフレンドだとわかったら、同じ鍵で `/wrist/history` を送り直し、Frame 側で印を書き足します。履歴の 1 行には `"f":"1"` が付きます（フレンドでなければ書かない）。印は消えません
- `-RefreshFriends` で今すぐ取り直して件数だけ表示。`--dump-png … --friend` でフレンドのカードを PNG にできます

## 拾うログの行

VRChat のログ出力を Full にすると出る行です（形式は [VRCX](https://github.com/vrcx-team/VRCX) の LogWatcher と同じ）。

| 種類 | 行（`YYYY.MM.DD HH:MM:SS Log - ` のあと） |
|---|---|
| 参加 | `[Behaviour] OnPlayerJoined 名前 (usr_…)` |
| 退出 | `[Behaviour] OnPlayerLeft 名前 (usr_…)` |
| ワールド | `[Behaviour] Entering Room: ワールド名` |
| ワールド ID | `[Behaviour] Destination set: wrld_…:…`（入る前）、`[Behaviour] Joining wrld_…:…`（今の VRChat は入った直後、古いログは入る前） |
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

Windows `%APPDATA%\vrc-wrist-notifier\config.json`: `frame_host`, `frame_port`, `log_dir`（空なら既定の場所）, `quiet_seconds`, `notify_join` / `notify_leave` / `notify_world` / `notify_invite` / `notify_friend` / `notify_other`、`backfill_history`（起動時に過去のログから履歴を送る、既定 true）、`friends` / `friends_refresh_days` / `friends_unknown_refresh_minutes` / `friends_file` / `wsl_distro`（フレンド、上を参照）。

## テスト

```sh
frame/build.sh /tmp/wrist-toast && /tmp/wrist-toast --self-test       # Frame 上で（g++・cairo・pango は SteamOS に入っている）
/tmp/wrist-toast --dump-png /tmp/t.png --kind invite --title 名前 --text 本文   # SteamVR なしで見た目を確認
```

```powershell
powershell -ExecutionPolicy Bypass -File tests\run-windows-tests.ps1   # 合成ログで検出結果を確認
powershell -ExecutionPolicy Bypass -File windows\vrc-wrist-notifier.ps1 -Replay tests\sample_output_log.txt -DryRun
```

`tests/sample_output_log.txt`、`tests/sample_history_log.txt`、`tests/sample_world_log.txt` は形式だけまねた合成データです（実在の人やワールドは入っていません）。

## 自分で OSC を送る

`/wrist/notify` に文字列 3 つ（種類、タイトル、本文）を送れば、VRChat 以外からでも表示できます。種類は `join` / `leave` / `world` / `invite` / `requestInvite` / `friendRequest` のほか、VRChat 以外の通知用に（0.2.1 から）:

| 種類 | 色 | アイコン |
|---|---|---|
| `alert` / `urgent` | 赤（背景も赤み、枠が太く、表示は 1.5 倍の長さ） | `!` |
| `warn` | 琥珀 | `⚠` |
| `care` / `info` | 青緑 | `♥` |

`基本.詳細`（例 `alert.temp`、`care.break`）と書くと、色は基本で、アイコンは詳細で決まります: `temp` 🌡、`battery` 🔋、`charge` ⚡、`break` ☕、`water` 💧。知らない種類は今までどおり青で表示します。[frame-care](https://github.com/cympfh/frame-care) はこの形で送ります。文字列 1 つなら本文だけ、2 つならタイトルと本文として扱います。

## リリース

Frame 上で `frame/build.sh` したものを `wrist-toast-linux-arm64` として GitHub Releases に置き、`SHA256SUMS` を添えます。インストーラーは sha256 と `--self-test` を確かめてから使い、だめなときはその場でビルドします。

## ライセンス

MIT。`vendor/openvr/openvr.h` は Valve の OpenVR SDK（BSD 3-Clause、`vendor/openvr/LICENSE`）。`third_party/frame-mic-tuner/vk_texture.*` は sasaken1102r さんの frame-mic-tuner から（MIT、`third_party/frame-mic-tuner/LICENSE`）。
