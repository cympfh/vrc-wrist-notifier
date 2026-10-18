#!/bin/bash
# SPDX-License-Identifier: MIT
# wrist-toast を Steam Frame の上でビルドする（g++、cairo、pango、Vulkan が要る。SteamOS には入っている）。
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="${1:-$here/wrist-toast}"
steamvr_lib=/opt/steamvr/bin/linuxarm64
tp="$here/../third_party/frame-mic-tuner"
g++ -std=c++17 -O2 -Wall -Wextra -o "$out" "$here/wrist-toast.cpp" "$tp/vk_texture.cpp" \
    -I"$here" -I"$tp" -I"$here/../vendor/openvr" $(pkg-config --cflags pangocairo cairo vulkan) \
    -L"$steamvr_lib" -lopenvr_api -Wl,-rpath,"$steamvr_lib" \
    $(pkg-config --libs pangocairo cairo vulkan) -lgobject-2.0
echo "$out"
