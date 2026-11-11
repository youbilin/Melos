"""Generate the Windows icon matching assets/melos.svg (requires Pillow)."""
from pathlib import Path
from PIL import Image, ImageDraw

root = Path(__file__).resolve().parent.parent
scale = 4
image = Image.new("RGBA", (256 * scale, 256 * scale))
draw = ImageDraw.Draw(image)
draw.rounded_rectangle((0, 0, 256 * scale - 1, 256 * scale - 1), radius=56 * scale, fill="#151518")
points = [(72, 172), (72, 68), (128, 132), (184, 68), (184, 172)]
draw.line([(x * scale, y * scale) for x, y in points], fill="#f5f5f7", width=20 * scale, joint="curve")
for x, y in points:
    draw.ellipse(((x - 10) * scale, (y - 10) * scale, (x + 10) * scale, (y + 10) * scale), fill="#f5f5f7")
for x in (72, 184):
    draw.ellipse(((x - 18) * scale, 166 * scale, (x + 18) * scale, 202 * scale), fill="#f5f5f7")
image.resize((256, 256), Image.Resampling.LANCZOS).save(root / "assets/melos.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
