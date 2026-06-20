"""生成 pvm GUI 所需的应用图标（一次性脚本）。"""
from PIL import Image, ImageDraw, ImageFont
import os

here = os.path.dirname(os.path.abspath(__file__))
icons = os.path.join(here, "icons")
os.makedirs(icons, exist_ok=True)

S = 512
img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)
d.rounded_rectangle([24, 24, S - 24, S - 24], radius=96, fill=(79, 70, 229, 255))
d.rounded_rectangle([24, 24, S - 24, S - 24], radius=96, outline=(255, 255, 255, 60), width=6)

font = None
for fp in (r"C:\Windows\Fonts\segoeuib.ttf", r"C:\Windows\Fonts\arialbd.ttf"):
    try:
        font = ImageFont.truetype(fp, 200)
        break
    except Exception:
        pass
if font is None:
    font = ImageFont.load_default()

d.text((S / 2, S / 2 - 12), "pvm", fill=(255, 255, 255, 255), anchor="mm", font=font)

img.save(os.path.join(icons, "icon.png"))
for s in (32, 128, 256):
    img.resize((s, s), Image.LANCZOS).save(os.path.join(icons, f"{s}x{s}.png"))
img.resize((256, 256), Image.LANCZOS).save(os.path.join(icons, "128x128@2x.png"))
img.save(
    os.path.join(icons, "icon.ico"),
    sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
)
print("icons generated at", icons)
