#!/bin/bash
# SPDX-License-Identifier: MIT
# vrc-wrist-notifier（Steam Frame 側）を入れる・外す。Frame の Konsole で、ふつうのユーザーで実行する（sudo 不要）。
#
#   curl -fsSL https://raw.githubusercontent.com/cympfh/vrc-wrist-notifier/main/frame/install.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/cympfh/vrc-wrist-notifier/main/frame/install.sh | bash -s -- --uninstall
#
# 入れるもの: ~/.local/opt/vrc-wrist-notifier/（本体 wrist-toast）、~/.config/systemd/user/vrc-wrist-notifier.service、
#             ~/.config/vrc-wrist-notifier/config.ini（無いときだけサンプルからコピー）
# 本体は GitHub Releases のビルド済みファイル（sha256 を確認）。使えないときはその場でビルドする。
# 自動起動は勝手にオンにしない。SteamVR などは再起動しない。
# 外すとき（--uninstall）は消さずに ~/.local/share/vrc-wrist-notifier-uninstalled/<日時>/ に移す。設定ファイルは残す。
set -euo pipefail

name=vrc-wrist-notifier
repo=cympfh/$name
ref=${FRAME_TOOL_REF:-main}
dest="$HOME/.local/opt/$name"
config_home="${XDG_CONFIG_HOME:-$HOME/.config}"
unit_dir="$config_home/systemd/user"
unit="$name.service"
conf_dir="$config_home/$name"
asset=wrist-toast-linux-arm64

real="/run/user/$(id -u)"
if [[ ! -S "${XDG_RUNTIME_DIR:-}/systemd/private" && -S "$real/systemd/private" ]]; then
    export XDG_RUNTIME_DIR="$real" DBUS_SESSION_BUS_ADDRESS="unix:path=$real/bus"
fi

[[ "$(id -u)" != 0 ]] || { echo "root では動かしません。sudo なしで実行してください。" >&2; exit 1; }

if [[ "${1:-}" == "--uninstall" ]]; then
    systemctl --user disable --now "$unit" 2>/dev/null || true
    aside="$HOME/.local/share/$name-uninstalled/$(date +%Y%m%d-%H%M%S)"
    mkdir -p "$aside"
    if [[ -e "$dest" ]]; then mv "$dest" "$aside/"; fi
    if [[ -e "$unit_dir/$unit" ]]; then mv "$unit_dir/$unit" "$aside/"; fi
    systemctl --user daemon-reload || true
    echo "$name を外しました。"
    echo "  外したファイルは $aside に移してあります（要らなければ消してOK）。"
    echo "  設定ファイル $conf_dir はそのまま残しています。"
    exit 0
fi

cache="${XDG_CACHE_HOME:-$HOME/.cache}"
mkdir -p "$cache"
tmp="$(mktemp -d "$cache/$name-install.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT

# ソースの出どころ: このスクリプトがリポジトリの frame/ にあればそれ、無ければ GitHub から tar.gz（git は使わない）
src=""
if [[ -n "${BASH_SOURCE[0]:-}" && -f "${BASH_SOURCE[0]}" ]]; then
    d="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
    [[ -f "$d/frame/wrist-toast.cpp" ]] && src="$d"
fi
if [[ -z "$src" ]]; then
    if [[ -n "${FRAME_TOOL_TARBALL:-}" ]]; then
        cp "$FRAME_TOOL_TARBALL" "$tmp/src.tar.gz"
    else
        echo "ダウンロード中: https://github.com/$repo ($ref)"
        curl -fsSL "https://codeload.github.com/$repo/tar.gz/$ref" -o "$tmp/src.tar.gz"
    fi
    mkdir -p "$tmp/src"
    tar xzf "$tmp/src.tar.gz" -C "$tmp/src" --strip-components=1
    src="$tmp/src"
fi

# 本体: ビルド済みを試して、だめならビルドする
bin="$tmp/wrist-toast"
got=""
if [[ "$(uname -m)" == aarch64 && -z "${VRC_WRIST_BUILD:-}" ]]; then
    base="https://github.com/$repo/releases/latest/download"
    if curl -fsL "$base/$asset" -o "$bin" && curl -fsL "$base/SHA256SUMS" -o "$tmp/SHA256SUMS"; then
        want="$(awk -v a="$asset" '$2==a{print $1}' "$tmp/SHA256SUMS")"
        have="$(sha256sum "$bin" | cut -d' ' -f1)"
        if [[ -n "$want" && "$want" == "$have" ]]; then
            chmod 755 "$bin"
            if "$bin" --self-test >/dev/null 2>&1; then got="ビルド済み（GitHub Releases、sha256 確認済み）"; fi
        else
            echo "ビルド済みファイルの sha256 が合わないので使いません。"
        fi
    else
        echo "ビルド済みファイルを取得できませんでした。"
    fi
fi
if [[ -z "$got" ]]; then
    echo "その場でビルドします（数十秒）..."
    bash "$src/frame/build.sh" "$bin" >/dev/null
    "$bin" --self-test >/dev/null
    got="この Frame でビルド"
fi

mkdir -p "$dest" "$unit_dir" "$conf_dir"
install -m755 "$bin" "$dest/wrist-toast.new" && mv -f "$dest/wrist-toast.new" "$dest/wrist-toast"
for f in frame/config.example.ini README.md LICENSE; do
    install -m644 "$src/$f" "$dest/$(basename "$f").new" && mv -f "$dest/$(basename "$f").new" "$dest/$(basename "$f")"
done
if [[ ! -f "$conf_dir/config.ini" ]]; then
    cp "$src/frame/config.example.ini" "$conf_dir/config.ini"
fi
install -m644 "$src/frame/$unit" "$unit_dir/$unit"
systemctl --user daemon-reload
systemctl --user try-restart "$unit" 2>/dev/null || true

cat <<MSG

vrc-wrist-notifier（Frame 側）を入れました。本体: $got
  オンにする:  systemctl --user enable --now $name
  オフにする:  systemctl --user disable --now $name
  表示テスト:  ~/.local/opt/$name/wrist-toast --dump-png /tmp/toast.png   （SteamVR なしで見た目を PNG に）
  設定:        $conf_dir/config.ini
MSG
