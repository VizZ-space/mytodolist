# -*- coding: utf-8 -*-
"""
生成「我的任务台」应用图标。
遵循 macOS Big Sur+ 图标规范：1024 画布中，图形本体约 824（四周留 100 给投影），
圆角半径约本体的 22.5%，自带顶部光泽与柔和投影。
"""
import os
import subprocess
from PIL import Image, ImageDraw, ImageFilter, ImageChops

S = 1024
M = 100                      # 边距：给投影留白，保证 Dock 里与其他 App 视觉等大
BOX = (M, M, S - M, S - M)   # 本体 824x824
R = 186                      # 圆角半径 ≈ 824 * 0.225

TOP = (86, 170, 255)         # 顶部亮蓝
BOT = (10, 90, 216)          # 底部深蓝


def rounded_mask(size=S, box=BOX, radius=R):
    m = Image.new("L", (size, size), 0)
    ImageDraw.Draw(m).rounded_rectangle(box, radius=radius, fill=255)
    return m


def vertical_gradient(top, bot):
    g = Image.new("RGB", (1, S))
    for y in range(S):
        t = y / (S - 1)
        g.putpixel((0, y), tuple(int(top[i] + (bot[i] - top[i]) * t) for i in range(3)))
    return g.resize((S, S)).convert("RGBA")


def draw_check(layer, color, offset=(0, 0), width=88):
    """画对勾：三点折线 + 圆形端点补齐圆头"""
    ox, oy = offset
    pts = [(322 + ox, 528 + oy), (452 + ox, 656 + oy), (706 + ox, 388 + oy)]
    d = ImageDraw.Draw(layer)
    d.line(pts, fill=color, width=width, joint="curve")
    r = width // 2
    for (x, y) in (pts[0], pts[-1]):
        d.ellipse([x - r, y - r, x + r, y + r], fill=color)
    return layer


def build():
    mask = rounded_mask()

    # 1) 渐变本体
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))

    # 1a) 外投影（macOS 图标自带的柔和下投影）
    shadow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle(
        (M, M + 16, S - M, S - M + 16), radius=R, fill=(0, 0, 0, 78)
    )
    shadow = shadow.filter(ImageFilter.GaussianBlur(26))
    img = Image.alpha_composite(img, shadow)

    body = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    body.paste(vertical_gradient(TOP, BOT), (0, 0), mask)

    # 2) 顶部光泽：上半部叠一层由白到透明的渐变，限制在圆角内
    hl = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(hl)
    span = 400
    for y in range(M, M + span):
        a = int(52 * (1 - (y - M) / span) ** 1.7)
        d.line([(0, y), (S, y)], fill=(255, 255, 255, a))
    hl.putalpha(ImageChops.multiply(hl.split()[3], mask))
    body = Image.alpha_composite(body, hl)

    # 3) 底部内发光，让本体不至于死板
    gl = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(gl)
    for y in range(S - M - 240, S - M):
        a = int(26 * ((y - (S - M - 240)) / 240) ** 1.5)
        d.line([(0, y), (S, y)], fill=(255, 255, 255, a))
    gl.putalpha(ImageChops.multiply(gl.split()[3], mask))
    body = Image.alpha_composite(body, gl)

    # 4) 对勾投影 + 对勾本体
    cs = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    draw_check(cs, (0, 40, 96, 120), offset=(0, 12))
    cs = cs.filter(ImageFilter.GaussianBlur(14))
    body = Image.alpha_composite(body, cs)

    ck = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    draw_check(ck, (255, 255, 255, 255))
    body = Image.alpha_composite(body, ck)

    # 5) 一道细边，提升边缘锐利度
    edge = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(edge).rounded_rectangle(
        BOX, radius=R, outline=(255, 255, 255, 46), width=3
    )
    body = Image.alpha_composite(body, edge)

    return Image.alpha_composite(img, body)


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    icons = os.path.join(here, "src-tauri", "icons")
    os.makedirs(icons, exist_ok=True)

    master = build()
    master_path = os.path.join(icons, "icon.png")
    master.save(master_path)
    print("母版 1024 已生成")

    # PNG 各尺寸（Tauri / Windows Store 用）
    sizes = {
        "32x32.png": 32,
        "64x64.png": 64,
        "128x128.png": 128,
        "128x128@2x.png": 256,
        "Square30x30Logo.png": 30,
        "Square44x44Logo.png": 44,
        "Square71x71Logo.png": 71,
        "Square89x89Logo.png": 89,
        "Square107x107Logo.png": 107,
        "Square142x142Logo.png": 142,
        "Square150x150Logo.png": 150,
        "Square284x284Logo.png": 284,
        "Square310x310Logo.png": 310,
        "StoreLogo.png": 50,
    }
    for name, px in sizes.items():
        master.resize((px, px), Image.LANCZOS).save(os.path.join(icons, name))
    print("派生 %d 个 PNG 尺寸" % len(sizes))

    # .ico（Windows）
    master.resize((256, 256), Image.LANCZOS).save(
        os.path.join(icons, "icon.ico"),
        sizes=[(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )
    print("icon.ico 已生成")

    # .icns（macOS）—— 走标准 iconset + iconutil
    iconset = os.path.join(here, "icon.iconset")
    if os.path.isdir(iconset):
        subprocess.run(["rm", "-rf", iconset], check=False)
    os.makedirs(iconset)
    for base in (16, 32, 128, 256, 512):
        master.resize((base, base), Image.LANCZOS).save(
            os.path.join(iconset, "icon_%dx%d.png" % (base, base))
        )
        master.resize((base * 2, base * 2), Image.LANCZOS).save(
            os.path.join(iconset, "icon_%dx%d@2x.png" % (base, base))
        )
    r = subprocess.run(
        ["iconutil", "-c", "icns", iconset, "-o", os.path.join(icons, "icon.icns")],
        capture_output=True, text=True,
    )
    print("icon.icns:", "成功" if r.returncode == 0 else "失败 " + r.stderr)
    subprocess.run(["rm", "-rf", iconset], check=False)

    # 预览图，方便肉眼确认
    preview = Image.new("RGBA", (760, 300), (245, 245, 247, 255))
    x = 40
    for px in (256, 128, 64, 32, 16):
        ic = master.resize((px, px), Image.LANCZOS)
        preview.alpha_composite(ic, (x, 40 + (256 - px) // 2))
        x += px + 34
    pv = os.path.join(here, "icon-preview.png")
    preview.convert("RGB").save(pv)
    print("预览图:", pv)


if __name__ == "__main__":
    main()
