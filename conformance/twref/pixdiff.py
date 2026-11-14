# Pixel difference between same-named PNGs in two directories.
#   python3 -P pixdiff.py <ref dir> <got dir> <name.png>...
# Prints the worst frame's mean absolute difference (0-255) and the share of
# pixels off by more than 32 in any channel, plus the mean over all frames.
import sys
from PIL import Image, ImageChops, ImageStat

ref, got = sys.argv[1], sys.argv[2]
rows = []
for name in sys.argv[3:]:
    a = Image.open(f'{ref}/{name}').convert('RGB')
    b = Image.open(f'{got}/{name}').convert('RGB')
    if b.size != a.size:
        b = b.resize(a.size)
    d = ImageChops.difference(a, b)
    mean = sum(ImageStat.Stat(d).mean) / 3
    over = ImageStat.Stat(d.convert('L').point(lambda v: 255 if v > 32 else 0)).mean[0] / 255 * 100
    frame = name.rsplit('-f', 1)[-1].split('.')[0].lstrip('0')
    rows.append((mean, over, frame))
worst = max(rows)
avg = sum(r[0] for r in rows) / len(rows)
print(f'worst frame {worst[2]}: mean {worst[0]:.2f} >32 {worst[1]:.2f}% | avg mean {avg:.2f} over {len(rows)} shots')
