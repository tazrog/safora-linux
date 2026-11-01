#!/usr/bin/env python3
"""Generate the Safora logo, wallpapers and boot-splash images (SVG), then render PNGs with rsvg-convert."""
import subprocess, pathlib

here = pathlib.Path(__file__).parent
prof = here.parent / "profile/airootfs/usr/share"

SHIELD = "M0 -1 C0.55 -0.78 0.95 -0.72 1 -0.7 C1 0.05 0.7 0.65 0 1 C-0.7 0.65 -1 0.05 -1 -0.7 C-0.95 -0.72 -0.55 -0.78 0 -1 Z"

def shield(scale, fill="none", stroke=None, sw=2, op=1):
    s = f'stroke="{stroke}" stroke-width="{sw/scale}"' if stroke else ""
    return f'<path transform="scale({scale})" d="{SHIELD}" fill="{fill}" {s} opacity="{op}"/>'

logo = f'''<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512">
  <defs>
    <linearGradient id="g2" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#4a3a6e"/><stop offset="1" stop-color="#241a3a"/></linearGradient>
    <linearGradient id="cy" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#7ff0ea"/><stop offset="1" stop-color="#2bb8c4"/></linearGradient>
  </defs>
  <g transform="translate(256 262)">
    <g transform="scale(215)"><path d="{SHIELD}" fill="url(#g2)"/></g>
    <g transform="scale(215)"><path transform="scale(0.8)" d="{SHIELD}" fill="none" stroke="#ece8f5" stroke-opacity="0.5" stroke-width="0.035"/></g>
    <path d="M 62 -92 C -10 -118 -78 -86 -66 -40 C -56 4 56 -6 66 48 C 78 104 -2 122 -68 92" fill="none" stroke="url(#cy)" stroke-width="26" stroke-linecap="round"/>
  </g>
</svg>'''

def wall(name, bg, glow1, glow2, line, ring, op_ring):
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="2560" height="1440" viewBox="0 0 2560 1440">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{bg[0]}"/><stop offset="0.55" stop-color="{bg[1]}"/><stop offset="1" stop-color="{bg[2]}"/></linearGradient>
    <radialGradient id="a" cx="0.75" cy="0.28" r="0.55"><stop offset="0" stop-color="{glow1}" stop-opacity="0.38"/><stop offset="1" stop-color="{glow1}" stop-opacity="0"/></radialGradient>
    <radialGradient id="b" cx="0.15" cy="0.9" r="0.55"><stop offset="0" stop-color="{glow2}" stop-opacity="0.30"/><stop offset="1" stop-color="{glow2}" stop-opacity="0"/></radialGradient>
    <linearGradient id="r1" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{line}" stop-opacity="0.16"/><stop offset="1" stop-color="{line}" stop-opacity="0.02"/></linearGradient>
    <filter id="soft"><feGaussianBlur stdDeviation="1.5"/></filter>
  </defs>
  <rect width="2560" height="1440" fill="url(#bg)"/>
  <rect width="2560" height="1440" fill="url(#a)"/>
  <rect width="2560" height="1440" fill="url(#b)"/>
  <g filter="url(#soft)">
    <path d="M0 1010 L420 800 L760 960 L1180 700 L1620 930 L2020 740 L2560 980 L2560 1440 L0 1440 Z" fill="url(#r1)"/>
    <path d="M0 1160 L520 960 L980 1130 L1500 930 L2040 1120 L2560 1000 L2560 1440 L0 1440 Z" fill="url(#r1)"/>
    <path d="M0 1300 L640 1160 L1260 1290 L1900 1150 L2560 1260 L2560 1440 L0 1440 Z" fill="url(#r1)"/>
  </g>
  <g transform="translate(1280 640)" fill="none" stroke="{ring}" opacity="{op_ring}">
    <g transform="scale(330)"><path d="{SHIELD}" stroke-width="0.006"/></g>
    <g transform="scale(420)"><path d="{SHIELD}" stroke-width="0.004"/></g>
    <g transform="scale(520)"><path d="{SHIELD}" stroke-width="0.003"/></g>
  </g>
</svg>'''

def small(name, content):
    (here / name).write_text(content)

dark_default = wall("default", ["#0b0814", "#17102a", "#21183a"], "#3dd6d0", "#8a63d9", "#7ff0ea", "#ece8f5", 0.12)
dark_aurora  = wall("aurora",  ["#090714", "#130f2e", "#1d1a48"], "#8a63d9", "#3dd6d0", "#b79cff", "#b79cff", 0.10)
light_dawn   = wall("dawn",    ["#f4f1fa", "#e6e0f2", "#d6cde8"], "#3dd6d0", "#b79cff", "#4b3a7a", "#4b3a7a", 0.18)

svgs = {
    "logo.svg": logo,
    "wallpaper-safora.svg": dark_default,
    "wallpaper-aurora.svg": dark_aurora,
    "wallpaper-dawn.svg": light_dawn,
    "dot.svg": '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><circle cx="8" cy="8" r="6" fill="#3dd6d0"/></svg>',
    "bullet.svg": '<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14"><circle cx="7" cy="7" r="5" fill="#ece8f5"/></svg>',
    "entry.svg": '<svg xmlns="http://www.w3.org/2000/svg" width="380" height="48"><rect x="1.5" y="1.5" width="377" height="45" rx="12" fill="#1b1528" fill-opacity="0.92" stroke="#3dd6d0" stroke-opacity="0.7" stroke-width="2"/></svg>',
}
for n, c in svgs.items():
    (here / n).write_text(c)

def render(svg, png, w=None, h=None):
    cmd = ["rsvg-convert", str(here / svg), "-o", str(png)]
    if w: cmd += ["-w", str(w)]
    if h: cmd += ["-h", str(h)]
    subprocess.run(cmd, check=True)

bg = prof / "backgrounds/safora"
bg.mkdir(parents=True, exist_ok=True)
for old in bg.glob("*.png"): old.unlink()
render("wallpaper-safora.svg", bg / "safora-default.png")
render("wallpaper-aurora.svg", bg / "safora-aurora.png")
render("wallpaper-dawn.svg", bg / "safora-dawn.png")
ply = prof / "plymouth/themes/safora"
render("logo.svg", ply / "logo.png", 180, 180)
render("logo.svg", here / "logo-512.png")
for n in ("dot", "bullet", "entry"):
    render(n + ".svg", ply / (n + ".png"))
print("art done")
