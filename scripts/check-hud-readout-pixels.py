"""Check screenshot pixels, not just computed CSS, after the six-layout browser run."""
import json
import sys
from pathlib import Path
from PIL import Image

folder = Path(sys.argv[1])
checked = 0
layouts = json.loads((folder / "layout.json").read_text())["results"]
assert {(x['tag'], x['theme']) for x in layouts} == {
    (tag, theme) for tag in ['desktop', 'laptop', 'phone'] for theme in ['dark', 'light']
}, 'Six completed viewport/theme captures are required'
for layout in layouts:
    if layout["theme"] != "light":
        continue
    values = layout['readouts']
    assert len(values) == 5, f'{layout["tag"]}: five readouts required'
    for i, value in enumerate(values):
        # Element captures scroll each phone instrument into view before taking real pixels.
        crop = Image.open(folder / f'{layout["tag"]}-light-readout-{i}.png').convert('RGB')
        ink = sum(max(abs(channel - wanted) for channel, wanted in zip(pixel, (20, 35, 34))) < 30
                  for pixel in crop.getdata())
        assert ink >= 15, f'{layout["tag"]} {value["text"]}: no dark ink in rendered readout ({ink})'
        checked += 1
assert checked == 15, f"Only {checked} readouts checked"
print(f"PASS {checked} light readouts contain dark rendered ink")
