# 開発者向けメモ

## しくみ

```
VRChat ──書く──▶ output_log_*.txt ──読む──▶ vrc-wrist-notifier.exe (Windows)
                                                  │ UDP / OSC  /wrist/notify ,sss  kind title text
                                                  ▼
                                    wrist-toast (Steam Frame, port 39570) ──▶ SteamVR オーバーレイ
```

- Windows 側（0.5.0 から）: Rust のアプリ 1 つ（`windows/`、窓なし・システムトレイ常駐、二重起動しない）。0.4 までの PowerShell 版と同じふるまい・同じ送り方なので、Frame 側は新旧どちらとも組めます。最新の `output_log_*.txt` を 0.5 秒ごとに読み足し（共有モードで開くので VRChat の邪魔をしません）、新しいログファイルができたら乗り換えます。起動前の行は通知しません
- Frame 側: C++。cairo + pango（Noto Sans CJK JP）で 640×160 のカードを描き、OpenVR の `SetOverlayRaw` で表示します。`libopenvr_api.so` は SteamVR に入っているものを使います（RUNPATH `/opt/steamvr/bin/linuxarm64`）
- SteamVR が動いていないあいだは 5 秒ごとにつなぎ直し、届いた通知は捨てます

## 履歴とダッシュボードのパネル（0.2.0）

- Windows 側は、参加・退出・ワールドの通知に 4〜7 個目の文字列を付けます: 時刻（`yyyy-MM-dd HH:mm:ss`、VRChat のログの時刻＝PC の時刻）、ワールド名、重複を除く鍵（`時刻|種類|名前`）、`quiet`（通知は出さず履歴だけ。ワールドに入った直後の参加など）。時刻の無い通知（`--test`、0.1.0 の Windows 側）は履歴に残しません
- 起動時（`backfill_history`）と `--backfill` で、今ある `output_log_*.txt` から参加・退出・ワールドを `/wrist/history ,sssss 時刻 種類 名前 ワールド 鍵` で送ります（通知は出ない）。同じ鍵は Frame 側で 1 回だけ記録します
- Frame 側は `~/.local/state/vrc-wrist-notifier/history.jsonl` に 1 行 1 件で追記します（`{"t","k","n","w","key"}`）。2 MB を超えたら `history.1.jsonl` に回します（1 世代だけ残すので、合計でおよそ 4 MB まで）。このファイルはリポジトリには入りません
- パネルは `CreateDashboardOverlay`（キー `cympfh.vrc-wrist-notifier.history`、1200×800 px、幅 2.6 m、マウス入力、`SendVRSmoothScrollEvents`）。wrist-toast が SteamVR につないだときにその場で登録するので、SteamVR の再起動は要りません。wrist-toast が止まるとパネルも消えます
- 描き直すのはパネルが開いているときだけ。5 秒ごとに `FindOverlay` で自分のパネルが残っているか確かめ、消えていたら作り直します
- 画像は Vulkan のテクスチャ（`SetOverlayTexture`）で渡します。部品は [frame-mic-tuner](https://github.com/sasaken1102r/frame-mic-tuner) の `vk_texture.*`（MIT、`third_party/frame-mic-tuner/`）。`SetOverlayRaw` はちらつきとコンポジタの不調の疑いがあると同プロジェクトに記録があるため、Vulkan が使えないときだけの予備です
- `wrist-toast --probe` で、動いている wrist-toast のパネルが SteamVR にあるかを別のプロセスから確かめられます。`--dump-history-png out.png [--history-dir D] [--filter join|leave] [--scroll PX]` で、SteamVR なしでパネルを PNG にできます

## ワールド名と見張りの様子（0.3.0）

- Windows 側は今いるワールドの名前と ID（`wrld_…` だけ。インスタンスの部分は人の ID が入るので送らない）を覚え、`/wrist/notify` の 8 個目、`/wrist/history` の 6 個目に ID を付けます。古い Frame 側は余分な引数を読まないだけです
- ワールドはログのファイルごとに数えなおします（VRChat を起動しなおすと新しいファイル）。見張りを途中から始めたときは、今のログファイルを頭から読んで、最後に入ったワールドを覚えます（`--show-world` で同じ読み方の結果だけを表示）
- 履歴の 1 行に `"wid"` が増えました（無ければ書かない）。`w` と `wid` が無い古い行も読めます。同じ鍵の記録が来て、前の記録にワールドが無かったときは、ワールドを書き足した行を追記します（読み直すと後の行が勝つ）。過去のログの読み込みで、ワールド不明だった記録が埋まります
- パネルはワールドに入ったところで区切り、見出しに「入った時刻 〜 最後の出来事の時刻 ・ 来た人数」を出します。ワールドのわからない記録は、前の滞在から 30 分以上あいていたら「（ワールド不明）」として分けます
- Windows 側は 30 秒ごと（状態が変わったときはすぐ）に `/wrist/heartbeat ,ss 版 ログの状態` を送ります。ログの状態は今のログファイルの中身から判断します: Log / Debug の行があれば `full`、Error / Warning しか無ければ `errors`、まだ何も無ければ `unknown`、VRChat のログが無ければ `nolog`（`-Status` で同じ判断を表示）
- Frame 側は 90 秒以内に heartbeat が来ていれば「見張りが動いている」とし、履歴がまだ無いときのパネルに、見張りとデバッグログ（Full）のそれぞれが済んでいるか、済んでいなければ何をすればよいかを出します。heartbeat が無く時刻付きの通知だけが来るときは 0.3.0 より前の Windows 側とみなします
- `--dump-history-png out.png --history-dir 空のフォルダ --watcher on|old|off --vrc-log full|errors|unknown|nolog` で、その表示を PNG にできます

## フレンドの名前を黄色に（0.4.0）

- Windows 側は VRChat のフレンド一覧（`usr_…` の ID だけ）を `%LOCALAPPDATA%\vrc-wrist-notifier\friends.json`（`fetched_at`、`count`、`ids`）に置きます。キーは書きません
- 取り直すのは friends.json が `friends_refresh_minutes`（既定 60）分より古くなったときだけ（失敗したときも 1 時間はあける）。`GET https://api.vrchat.cloud/api/1/auth/user` を 1 回呼んで `friends` を使います（User-Agent `vrc-wrist-notifier/版 cympfh`）。取り直しは別の runspace で行うので、通知は止まりません
- キー（`auth` クッキーの値）は、Windows の環境変数 `VRCHAT_AUTH_KEY`（プロセス → ユーザー）、無ければ `wsl.exe -d <wsl_distro> --exec zsh -lic` で読みます。取り直すときだけ読み、表示・ログ・保存はしません。キーが無い・401 なら、起動しなおすまで自分からは取り直さず、今までどおり動きます
- `/wrist/notify` の 9 個目、`/wrist/history` の 7 個目が `"1"` ならフレンド。まとめカードでは先頭にフレンドを出し、「名前 さんほか N 人」の名前の部分だけ黄色にします
- 一覧に無い人が、次に取り直したあとでフレンドだとわかったら、同じ鍵で `/wrist/history` を送り直し、Frame 側で印を書き足します。履歴の 1 行には `"f":"1"` が付きます（フレンドでなければ書かない）。印は消えません
- `--refresh-friends`（トレイの「フレンド一覧を取り直す」）で今すぐ取り直して件数だけ表示。`--dump-png … --friend`（`--self`）でフレンド（自分）のカードを PNG にできます

## 通知する人（0.4.1）

- パネルの「通知設定」の帯で `全員` / `フレンドのみ` / `オフ` を選びます。Frame の `config.ini` の `notify_people`（`all` / `friends` / `off`）に書き、次の起動でも使います（その 1 行だけ書き換え、ほかの行とコメントは残す）
- 「フレンドのみ」が効くのは参加・退出のカードだけで、ワールド、招待、`alert` / `warn` / `care` などはそのまま出ます。「オフ」ではどの通知も出しません（0.4.3 から）。履歴にはどれでも全員残ります
- `/wrist/notify` の 10 個目に、そのカードに入っているフレンドの名前（タブ区切り）が付きます。「フレンドのみ」では、まとめカードもフレンドだけで数えなおします（2 人までは 1 人ずつ、3 人以上は「名前 さんほか N 人」）。10 個目が無い 0.4.0 の Windows 側からは 1 人のカードだけ判断します
- `/wrist/heartbeat` の 3 個目がフレンド一覧の状態（`ok` / `nokey` / `bad` / `none` / `off`）。「フレンドのみ」でフレンドがわからないときは、帯の右に短い注意を出します
- `--dump-history-png … --notify friends --friends-state nokey` で帯の見た目を PNG にできます

## VRCHAT_AUTH_KEY の確認（0.4.4）

- Windows 側は起動時に 1 回だけキーを確かめます。キーが無ければ API は呼ばず `nokey`、あればフレンド一覧の取り直しを兼ねて `GET /api/1/auth/user` を 1 回呼び、成功なら `ok`、401 やほかのエラーなら `bad`。結果は `/wrist/heartbeat` の 3 個目で送ります（0.4.1 からの値と同じなので、新旧どちらの組み合わせでも動きます）
- Frame 側は `nokey` / `bad` のときだけ、パネルの右下に小さく「VRCHAT_AUTH_KEY 未設定」「VRCHAT_AUTH_KEY が使えません」と出します。問題が無いとき、見張りが動いていないとき、古い Windows 側のときは何も出しません

## 自分の名前は水色（0.4.1）

- 自分の ID は VRChat のログの `[Behaviour] User Authenticated: 名前 (usr_…)` の行から取ります（API は使いません）。フレンド一覧を取ったときの `/auth/user` の `id` も friends.json の `self_id` に残し、ログに無いときに使います
- `/wrist/notify` の 9 個目と `/wrist/history` の 7 個目が `"self"` なら自分。`/wrist/notify` の 11 個目はそのカードに入っている自分の名前。履歴の 1 行には `"s":"1"`
- 自分は「全員」「フレンドのみ」のどちらでも通知し（「オフ」なら出さない）、まとめカードでは先頭に出します

## 見張りが止まったとき（0.4.5）

- Frame 側は heartbeat（30 秒ごと）が 3 回分、つまり 90 秒来ないと「止まっている」とし、パネルの右下に小さく「⚠ Windows 側の見張りが止まっています」と出します。wrist-toast を起動してから 90 秒は待ちます（それまでに来なければ同じく出す）
- 止まっているあいだはキーの様子がわからないので、キーの注意は出しません。heartbeat が届いているあいだは何も出しません
- 履歴がまだ無いときは、真ん中の「Windows 側の見張り：動いていません」が同じことを言うので、右下には出しません。0.3.0 より前の Windows 側（heartbeat は無いが通知は来る）は止まっているとはみなしません
- `--dump-history-png … --watcher down` で見た目を PNG にできます

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
| notify_people | all | 通知設定（`all` / `friends`: 参加・退出はフレンドと自分だけ / `off`: どの通知も出さない）。パネルで選ぶと書き換わる |

Windows `%APPDATA%\vrc-wrist-notifier\config.json`: `frame_host`, `frame_port`, `log_dir`（空なら既定の場所）, `quiet_seconds`, `notify_join` / `notify_leave` / `notify_world` / `notify_invite` / `notify_friend` / `notify_other`、`backfill_history`（起動時に過去のログから履歴を送る、既定 true）、`friends` / `friends_refresh_minutes` / `friends_file` / `wsl_distro`（フレンド、上を参照）。

## テスト

```sh
frame/build.sh /tmp/wrist-toast && /tmp/wrist-toast --self-test       # Frame 上で（g++・cairo・pango は SteamOS に入っている）
/tmp/wrist-toast --dump-png /tmp/t.png --kind invite --title 名前 --text 本文   # SteamVR なしで見た目を確認
```

```sh
cd windows && cargo test                                        # 合成ログ（tests/）で検出結果を確認（Linux / WSL でも回る）
cargo build --release --target x86_64-pc-windows-gnu            # WSL で Windows の exe を作る（mingw-w64 が要る）
```

```powershell
vrc-wrist-notifier.exe --replay tests\sample_output_log.txt --dry-run | Out-String   # 送らずに検出結果だけ表示
vrc-wrist-notifier.exe --test | Out-String                  # テスト通知を 1 件（ほか --backfill / --show-world / --status / --refresh-friends）
$env:VRC_WRIST_EXE = "…\vrc-wrist-notifier.exe"; .\windows\install.ps1   # 手元でビルドした exe をインストール
```

トレイのアプリなので、PowerShell から表示を見るときは `| Out-String` を付けます（付けないと終わるのを待たずに戻ります）。
動作ログは `%LOCALAPPDATA%\vrc-wrist-notifier\vrc-wrist-notifier.log`（1 MB で `.log.old` に回す。キーは書きません）。

## Windows 側のアプリ（0.5.0）

- `windows/src/watcher.rs`: ログの解析・まとめ・フレンド・heartbeat（OS に依らない。テストはここを通す）。`windows/src/tray.rs`: トレイ。`windows/src/friends_api.rs`: キーを読んで `/auth/user` を 1 回呼ぶ
- トレイの表示は問題があるときだけ: Frame に送れない（ホスト名が引けないなど）、VRChat のログ出力が Full でない、`VRCHAT_AUTH_KEY` 未設定・使えない。何も無ければツールチップは名前だけ。UDP なので「Frame が受け取ったか」まではわかりません（それは Frame 側の「Windows 側の見張りが止まっています」が受け持つ）
- 「フレンド一覧を取り直す」は、キーが効かなかった印も消して 1 回だけ取り直します（キーを新しくしたあと、起動しなおさずに試せる）
- アイコンは Frame 側のダッシュボードのアイコンと同じ絵（`windows/assets/icon.png` は `wrist-toast --dump-thumbnail-png` の出力、`make-icons.py` で .ico とトレイ用を作る。問題があるときは同じ絵に黄色の「!」）
- 自動起動はスタートアップのショートカット `vrc-wrist-notifier.lnk`（install.ps1 が作る）。二重起動は名前付きミューテックス `Local\vrc-wrist-notifier` で防ぎます
- リリース: `v*` のタグを push すると GitHub Actions（`.github/workflows/windows-release.yml`）が Ubuntu で `x86_64-pc-windows-gnu` にビルドし、`cargo test` のあと、そのタグのリリースに `vrc-wrist-notifier.exe` と `SHA256SUMS` を載せます（リリースが既にあれば差し替え。ノートは CHANGELOG.md にその版の節があればそれ、無ければ自動生成）。タグと `windows/Cargo.toml` の version が違うと止まります。Actions の画面から手で動かす（workflow_dispatch）と、ビルドとテストだけ。Frame 側は端末の上でビルドするので扱いません
- インストーラーは GitHub Releases の `vrc-wrist-notifier.exe`（あれば `SHA256SUMS` で確かめる）を置きます。0.4 までの PowerShell 版が動いていれば止め、`vrc-wrist-notifier.cmd`（自動起動）と `vrc-wrist-notifier.ps1` を片付けます

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
