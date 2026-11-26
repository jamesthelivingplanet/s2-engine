#!/usr/bin/env python3
"""Draw the tray icon (an original, Eva-inspired unit head) and render its PNGs.

Needs rsvg-convert. The PNGs are committed because the binary embeds them.
"""
import pathlib
import subprocess

HERE = pathlib.Path(__file__).parent
# state: (armour, outline and eyes)
STATES = {
    "stopped": ("#6b7280", "#e5e7eb"),
    "starting": ("#b45309", "#ffd21f"),
    "running": ("#7c3aed", "#8cff32"),
}
HEAD = "M32 2 L37 19 L48 17 L60 31 L53 36 L57 45 L43 62 L32 56 L21 62 L7 45 L11 36 L4 31 L16 17 L27 19 Z"
EYES = "M10 30 L29 36 L26 45 L13 39 Z M54 30 L35 36 L38 45 L51 39 Z"
MOUTH = "M24 50 L32 46 L40 50 L32 55 Z"

for state, (armour, light) in STATES.items():
    svg = HERE / f"{state}.svg"
    svg.write_text(
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">'
        f'<path d="{HEAD}" fill="{armour}" stroke="{light}" stroke-width="3.5" stroke-linejoin="round"/>'
        f'<path d="{EYES} {MOUTH}" fill="{light}"/></svg>\n'
    )
    for size in (22, 32, 48):
        out = HERE / f"{state}-{size}.png"
        subprocess.run(["rsvg-convert", "-w", str(size), "-h", str(size), str(svg), "-o", str(out)], check=True)
