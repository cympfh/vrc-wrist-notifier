# 開発者向けメモ

## しくみ（0.6.0 から）

```
VRChat ──書く──▶ output_log_*.txt ──読む──▶ vrc-wrist-notifier.exe（Windows、トレイ常駐）
                                                  │ 見張りのスレッド → チャンネル → VR のスレッド
                                                  ▼
                                   SteamVR（PC）のオーバーレイ ──ストリーミング──▶ ヘッドセット（Steam Frame など）
```

0.5 までは Steam Frame の上で wrist-toast（C++）が動き、Windows 側から UDP 39570 の OSC で受け取っていました。0.6.0 で全部を exe 1 つにまとめ、Frame 側・UDP・heartbeat はなくなりました。

- `windows/src/watcher.rs`: ログの解析・まとめ・フレンド・自分の退出時の片付けの除外。見つけたものを `Out::Toast` / `Out::History` としてチャンネルに流す（`--dry-run` では代わりに 1 行ずつ貯める。テストはここを通す）
- `windows/src/hub.rs`: 届いたものを履歴（`history.rs`）と通知の待ち行列に振り分ける。「通知設定」の絞り込み、1 秒 10 件・待ち 8 件まで
- `windows/src/toast.rs` / `panel.rs` / `draw.rs`: カード（640×160）とパネル（1200×800）をソフトウェアで描く（tiny-skia + fontdue。Windows は游ゴシック → メイリオ、記号は Segoe UI Symbol / Emoji）。OS に依らないので、見た目のテストも Linux で回る
- `windows/src/vr.rs`（Windows だけ）: OpenVR。`openvr_api.dll` は SteamVR に入っているもの（`%LOCALAPPDATA%\openvr\openvrpaths.vrpath` の runtime の `bin\win64`）を libloading で読むので、配るのは exe 1 つ。`VR_InitInternal(Overlay)` と `FnTable:IVROverlay_028` / `FnTable:IVRSystem_026`
- `windows/src/tray.rs`: トレイ。`friends_api.rs`: キーを読んで `/auth/user` を 1 回呼ぶ

## SteamVR とのつなぎ方

- SteamVR は自分から起動しない: `vrserver.exe` が動いているのを 5 秒ごとに確かめ、動いているときだけ初期化する（動いていないときに初期化すると SteamVR が起動してしまうため）
- `VREvent_Quit` を受けたら `AcknowledgeQuit_Exiting` して離れ、`vrserver.exe` がいったん消えるまではつなぎ直さない。トレイのアプリはそのまま動き続ける
- SteamVR が動いていないあいだの通知は捨てる（履歴には残る）
- 通知: オーバーレイ `cympfh.vrc-wrist-notifier`。頭（デバイス 0）からの相対で、前 `toast_distance_m`、下 `toast_y_m`、幅 `toast_width_m`。`toast_seconds`（急ぎは 1.5 倍）、前後 0.4 秒でフェード。画像は `SetOverlayRaw`（乗算していない RGBA）
- パネル: `CreateDashboardOverlay`（キー `cympfh.vrc-wrist-notifier.history`、名前「入退室の履歴」、幅 `panel_width_m`、マウス入力、`SendVRSmoothScrollEvents`）。サムネイルは `windows/assets/icon.png`（トレイ・exe と同じ絵）。マウスの y は左下が原点なので反転する。描き直すのは開いているときだけ。5 秒ごとに `FindOverlay` で残っているか確かめ、消えていたら作り直す
- .vrmanifest は登録しない（SteamVR から起動させる必要がない。自動起動はサインイン時のスタートアップのショートカット）

## 履歴

- `%LOCALAPPDATA%\vrc-wrist-notifier\history.jsonl`（`history_dir` で変えられる）に 1 行 1 件で追記: `{"t","k","n","w","key","wid","f","s"}`（`wid` / `f`="1"（フレンド）/ `s`="1"（自分）は無ければ書かない）。0.5 までの Frame の `~/.local/state/vrc-wrist-notifier/history.jsonl` と同じ形式なので、そのまま置けば読める
- 鍵は `時刻|種類|名前`。同じ鍵は 1 回だけ。前の記録にワールドやフレンドの印が無かったときは書き足した行を追記する（読み直すと後の行が勝つ）
- 2 MB を超えたら `history.1.jsonl` に回す（1 世代だけ）。メモリには 20000 件まで
- 起動時（`backfill_history`）と `--backfill` で、今ある `output_log_*.txt` から読み直す
- パネルはワールドに入ったところで区切り、見出しに「入った時刻 〜 最後の出来事の時刻 ・ 来た人数」。ワールドのわからない古い記録は、30 分以上あいていたら「（ワールド不明）」として分ける

## 問題だけを出す

- トレイ: VRChat のログ出力が Full でない、`VRCHAT_AUTH_KEY` 未設定・使えない、のときだけ黄色の「!」とメニュー・ツールチップに一言
- パネル: 右下に同じ注意を小さく。履歴がまだ無いときは、真ん中にデバッグログ（Full）が済んでいるかだけを出す
- 「フレンドのみ」でフレンド一覧が `off` / 取得中のときだけ、通知設定の下に一言

## フレンドと自分

- フレンド一覧（ID だけ）は `%LOCALAPPDATA%\vrc-wrist-notifier\friends.json`。`friends_refresh_minutes`（既定 60）分より古くなったときだけ、`GET https://api.vrchat.cloud/api/1/auth/user` を 1 回呼んで取り直す（裏のスレッド。失敗しても 1 時間はあける）
- キー（`auth` クッキーの値）は Windows の環境変数 `VRCHAT_AUTH_KEY`（プロセス → ユーザー）、無ければ `wsl.exe -d <wsl_distro> --exec zsh -lic` で読む。使うときだけ読み、表示・ログ・保存はしない
- 自分の ID はログの `[Behaviour] User Authenticated: 名前 (usr_…)` から。フレンドは黄色、自分は水色。まとめカードは先頭の名前だけ色を付ける
- 「フレンドのみ」が効くのは参加・退出だけ。まとめカードはフレンドと自分だけで数えなおす。「オフ」はどの通知も出さない。履歴はどれでも全員

## 拾うログの行

VRChat のログ出力を Full にすると出る行です（形式は [VRCX](https://github.com/vrcx-team/VRCX) の LogWatcher と同じ）。

| 種類 | 行（`YYYY.MM.DD HH:MM:SS Log - ` のあと） |
|---|---|
| 参加 | `[Behaviour] OnPlayerJoined 名前 (usr_…)` |
| 退出 | `[Behaviour] OnPlayerLeft 名前 (usr_…)` |
| ワールド | `[Behaviour] Entering Room: ワールド名` |
| ワールド ID | `[Behaviour] Destination set: wrld_…:…`（入る前）、`[Behaviour] Joining wrld_…:…`（今の VRChat は入った直後、古いログは入る前） |
| 招待など | `[API] Received Notification: <Notification from username:名前, sender user id:usr_… to … of type: invite, id: not_…, …>` |

自分がワールドを出るとき（ワールドの移動・インスタンスを出る）、VRChat は `Destination set` → `OnLeftRoom` → そこにいた全員の `OnPlayerLeft`（自分は最後）→ `Entering Room` の順に書きます。`OnLeftRoom`（と `VRCApplication: HandleApplicationQuit`）から次の `Entering Room` までの他人の `OnPlayerLeft` は片付けなので、通知も履歴もしません（0.5.1）。履歴には自分の退出だけが残り、それが「自分がワールドを出た」印になります。自分がいるあいだの他人の退出はそのまま。

通知の種類は `invite` / `requestInvite` / `friendRequest` を出し、それ以外は `notify_other` で切り替えます。同じ `not_…` は 1 回だけ出します。

## 設定

`%APPDATA%\vrc-wrist-notifier\config.json`（無くても既定値で動く。サンプルは `windows/config.example.json`）

| 項目 | 既定 | 意味 |
|---|---|---|
| notify_people | all | 通知設定（`all` / `friends` / `off`）。パネルで選ぶとこの項目だけ書き換わる |
| notify_join / leave / world / invite / friend / other | true（other は false） | 種類ごとのオン・オフ |
| batch_seconds / quiet_seconds | 3 / 10 | まとめる間隔 / ワールドに入った直後に参加を出さない秒数 |
| toast_seconds | 4 | 通知を出す秒数 |
| toast_distance_m / toast_y_m / toast_width_m | 0.8 / -0.18 / 0.30 | 通知の位置と大きさ |
| history_panel / panel_width_m | true / 2.6 | ダッシュボードの「入退室の履歴」 |
| history_dir | （空） | 履歴の場所。空なら `%LOCALAPPDATA%\vrc-wrist-notifier` |
| backfill_history | true | 起動時に過去のログから履歴を読む |
| friends / friends_refresh_minutes / friends_file / wsl_distro | true / 60 / （空） / Ubuntu | フレンド |
| log_dir | （空） | VRChat のログの場所。空なら既定 |

0.5 までの `frame_host` / `frame_port` は読み飛ばす。

## テスト・ビルド

```sh
cd windows && cargo test                                        # 合成ログ（tests/）・描画・履歴・設定（Linux / WSL でも回る）
cargo build --release --target x86_64-pc-windows-gnu            # WSL で Windows の exe を作る（mingw-w64 が要る）
```

```powershell
vrc-wrist-notifier.exe --replay tests\sample_output_log.txt --dry-run | Out-String   # 検出結果だけ表示
vrc-wrist-notifier.exe --dump-png out | Out-String          # カードと今の履歴のパネルを PNG に（Windows のフォントで）
vrc-wrist-notifier.exe --backfill | Out-String              # 過去のログから履歴に足す（ほか --show-world / --status / --refresh-friends）
$env:VRC_WRIST_EXE = "…\vrc-wrist-notifier.exe"; .\windows\install.ps1   # 手元でビルドした exe をインストール
```

テストは本物の `%APPDATA%` / `%LOCALAPPDATA%` に書かない（一時フォルダだけ）。トレイのアプリなので、PowerShell から表示を見るときは `| Out-String` を付ける。
動作ログは `%LOCALAPPDATA%\vrc-wrist-notifier\vrc-wrist-notifier.log`（1 MB で `.log.old` に回す。キーは書かない）。

`tests/sample_*.txt` は形式だけまねた合成データ（実在の人やワールドは入っていない）。

## リリース

`v*` のタグを push すると GitHub Actions（`.github/workflows/windows-release.yml`）が Ubuntu で `x86_64-pc-windows-gnu` にビルドし、`cargo test` のあと、そのタグのリリースに `vrc-wrist-notifier.exe` と `SHA256SUMS` だけを載せる。タグと `windows/Cargo.toml` の version が違うと止まる。workflow_dispatch はビルドとテストだけ。
インストーラーは Releases の exe を置き（`SHA256SUMS` で確かめる）、スタートアップに `vrc-wrist-notifier.lnk` を作る。二重起動は名前付きミューテックス `Local\vrc-wrist-notifier` で防ぐ。
アイコンは `windows/assets/icon.png`、`make-icons.py` で .ico とトレイ用（問題があるときの黄色い「!」付き）を作る。

## ライセンス

MIT
