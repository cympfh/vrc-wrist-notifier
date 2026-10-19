#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
# アプリのアイコン（exe・トレイ・ダッシュボード）を icon.png から作る（Pillow が要る）。
#   → icon.ico（exe）、icon-32.rgba（トレイ）、icon-warn-32.rgba（問題があるとき: 同じ絵に黄色の「!」）
#   cd windows/assets && python3 make-icons.py
from PIL import Image, ImageDraw

src = Image.open("icon.png").convert("RGBA")
S = src.width

warn = src.copy()
d = ImageDraw.Draw(warn)
r0 = S * 0.56                       # 右下に小さく
d.ellipse((r0, r0, S, S), fill=(18, 20, 26, 255))
p = S * 0.035
d.ellipse((r0 + p, r0 + p, S - p, S - p), fill=(255, 196, 0, 255))
cx, w = (r0 + S) / 2, S * 0.055
d.rounded_rectangle((cx - w / 2, r0 + S * 0.08, cx + w / 2, r0 + S * 0.28), radius=w / 2, fill=(26, 20, 0, 255))
d.ellipse((cx - w * 0.6, r0 + S * 0.32, cx + w * 0.6, r0 + S * 0.32 + w * 1.2), fill=(26, 20, 0, 255))

src.save("icon.ico", sizes=[(16, 16), (20, 20), (24, 24), (32, 32), (48, 48), (64, 64), (256, 256)])
for name, im in (("icon", src), ("icon-warn", warn)):
    with open(f"{name}-32.rgba", "wb") as f:
        f.write(im.resize((32, 32), Image.LANCZOS).tobytes())
warn.save("/tmp/icon-warn.png")
