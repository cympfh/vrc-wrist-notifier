# vrc-wrist-notifier

VRChat で **だれかが来た・帰った、ワールドを移動した、招待やフレンド申請が届いた** ときに、
VR の視界に小さなカードを数秒だけ出します。メニューを開かなくても気づけます。
カードは目の前の少し下に出ます。

![表示の例](docs/toast-join.png)
![表示の例](docs/toast-invite.png)

あとから **どのワールドで、だれがいつ来て帰ったか** を、SteamVR のダッシュボードで見返せます。

![入退室の履歴](docs/history.png)

VRChat を動かしている Windows PC に入れるだけです（SteamVR で表示するので、Steam Frame などにはストリーミングでそのまま届きます）。
配るのは `vrc-wrist-notifier.exe` 1 つだけです。

## インストール

PowerShell に貼り付けて Enter:

```powershell
irm https://raw.githubusercontent.com/cympfh/vrc-wrist-notifier/main/windows/install.ps1 | iex
```

すぐに起動し、次からはサインインしたときに自動で起動します。窓は出ず、タスクバーの右下（通知領域）にアイコンが入ります。

手で入れたいときは、[Releases](https://github.com/cympfh/vrc-wrist-notifier/releases/latest) から `vrc-wrist-notifier.exe` をダウンロードして、好きな場所でダブルクリックしてもかまいません（自動起動はしません）。

## VRChat の準備

VRChat のクイックメニュー → 設定 → **Debug**（デバッグ）のログ出力を **Full** にしてください。
Errors Only のままだと参加・退出がログに出ないので、通知できません。OSC の設定は要りません。

## 使い方

- SteamVR が動いているあいだだけ表示します。SteamVR を自分から起動することはありません
- ダッシュボード（メニューボタン）の下の並びの **「入退室の履歴」** で、新しい順の一覧が見られます。ワールドごとにまとまっています
- 一覧の上の **通知設定** で **全員 / フレンドのみ / オフ** を選べます（オフならすべての通知が止まります。履歴には全員残ります）
- トレイのアイコンを右クリックすると「設定…」「テスト通知（VR に出す）」「フレンド一覧を取り直す」「ログを開く」「終了」ができます
- ふだんは何も言いません。問題があるときだけ、アイコンに黄色の「!」が付き、何が問題かが出ます

<a id="vrchat-key"></a>
## フレンドを黄色にする（なくても使えます）

VRChat のキーを入れておくと、フレンドの名前が黄色、自分は水色になります。

1. ブラウザで [vrchat.com](https://vrchat.com/home) にログインします
2. F12 で開発者ツールを開き、Chrome / Edge なら「Application」（日本語表示なら「アプリケーション」）→「Cookies」→ `https://vrchat.com`、Firefox なら「ストレージ」→「Cookie」を開きます
3. 名前が `auth` の行の値（`authcookie_` で始まる文字列）をコピーします
4. トレイのアイコンを右クリック →「**設定…**」→ 貼り付けて「**保存**」。すぐに使われます（再起動は要りません）

- キーはパスワードと同じです。人に見せたり、チャットやスクリーンショットに載せたりしないでください
- キーは暗号化して、この PC のこの Windows ユーザーだけが読める形で保存します（`%LOCALAPPDATA%\vrc-wrist-notifier\auth.bin`）。ログにも残しません。「消す」でいつでも消せます
- ブラウザで VRChat からログアウトすると、そのキーは使えなくなります。そのときは、もう一度ログインしてキーを入れなおしてください
- 0.6 までに環境変数 `VRCHAT_AUTH_KEY` を設定していた人は、もう使わないので消してください（平文のまま残っています）。PowerShell に `[Environment]::SetEnvironmentVariable('VRCHAT_AUTH_KEY', $null, 'User')` を貼り付けて Enter で消せます。WSL の `~/.zshrc` などに書いたものも消してください
- フレンド一覧を取りに行くのは 1 時間に 1 回までです。手元に残すのは `%LOCALAPPDATA%\vrc-wrist-notifier\friends.json`（フレンドと自分の ID だけ）です

## アンインストール

PowerShell に貼り付けて Enter:

```powershell
irm https://raw.githubusercontent.com/cympfh/vrc-wrist-notifier/main/windows/uninstall.ps1 | iex
```

アプリを止めて自動起動をやめます。ファイルと履歴は `%LOCALAPPDATA%\vrc-wrist-notifier-uninstalled\` に移すだけで、消しません。

### 0.5 までの Frame 側を入れていた人へ

0.6 から Steam Frame には何も入れません。Frame の Konsole で次を貼り付けると、古い Frame 側を消せます（履歴が要るなら、先に `~/.local/state/vrc-wrist-notifier/history.jsonl` を PC の `%LOCALAPPDATA%\vrc-wrist-notifier\` にコピーしてください。形式は同じです）。

```sh
systemctl --user disable --now vrc-wrist-notifier; rm -rf ~/.config/systemd/user/vrc-wrist-notifier.service ~/.local/opt/vrc-wrist-notifier ~/.config/vrc-wrist-notifier ~/.local/state/vrc-wrist-notifier; systemctl --user daemon-reload
```

<details>
<summary>くわしい情報</summary>

- 設定: `%APPDATA%\vrc-wrist-notifier\config.json`（無くても動きます。項目は [windows/config.example.json](windows/config.example.json)）
- 履歴: `%LOCALAPPDATA%\vrc-wrist-notifier\history.jsonl`。人の名前が入るので、どこにも送りません
- 起動すると、PC に残っている VRChat のログからも履歴を読み込みます（同じものは 1 回だけ）
- ワールドに入った直後の 10 秒は、元からいた人の「参加」を出しません
- 3 人以上がまとめて参加・退出したときは「Mint さんほか 4 人が参加」のように 1 枚にまとめます
- 自分がワールドを出たときの、ほかの人の「退出」は出しません
- VRChat の設定やファイルは変更しません。管理者権限も要りません
- 開発者向け: [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md)

</details>
