#!/usr/bin/env python3
"""Generate the Steady logo, wallpapers and boot-splash images (SVG), then render PNGs with rsvg-convert."""
import subprocess, pathlib

here = pathlib.Path(__file__).parent
prof = here.parent / "profile/airootfs/usr/share"

SHIELD = "M0 -1 C0.55 -0.78 0.95 -0.72 1 -0.7 C1 0.05 0.7 0.65 0 1 C-0.7 0.65 -1 0.05 -1 -0.7 C-0.95 -0.72 -0.55 -0.78 0 -1 Z"

def shield(scale, fill="none", stroke=None, sw=2, op=1):
    s = f'stroke="{stroke}" stroke-width="{sw/scale}"' if stroke else ""
    return f'<path transform="scale({scale})" d="{SHIELD}" fill="{fill}" {s} opacity="{op}"/>'

logo = f'''<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512">
  <defs>
    <linearGradient id="g2" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#41576b"/><stop offset="1" stop-color="#1d2a37"/></linearGradient>
    <linearGradient id="amber" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#f7c968"/><stop offset="1" stop-color="#e9a22f"/></linearGradient>
  </defs>
  <g transform="translate(256 262)">
    <g transform="scale(215)"><path d="{SHIELD}" fill="url(#g2)"/></g>
    <g transform="scale(215)"><path transform="scale(0.8)" d="{SHIELD}" fill="none" stroke="#e6edf3" stroke-opacity="0.55" stroke-width="0.035"/></g>
    <rect x="-96" y="-70" width="192" height="34" rx="17" fill="url(#amber)"/>
    <rect x="-96" y="-12" width="192" height="34" rx="17" fill="#e6edf3" fill-opacity="0.9"/>
    <rect x="-60" y="46" width="120" height="34" rx="17" fill="url(#amber)"/>
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

dark_default = wall("default", ["#080c11", "#101822", "#18232f"], "#f2b84b", "#5b86b0", "#f7c968", "#e6edf3", 0.12)
dark_aurora  = wall("aurora",  ["#070b12", "#0f1a2c", "#1b2a45"], "#6aa8ff", "#f2b84b", "#9cc4ff", "#9cc4ff", 0.10)
light_dawn   = wall("dawn",    ["#f6f2ea", "#ebe4d6", "#dcd3c2"], "#f2b84b", "#9bb8d3", "#7a5a1a", "#5b4a2a", 0.18)

svgs = {
    "logo.svg": logo,
    "wallpaper-steady.svg": dark_default,
    "wallpaper-aurora.svg": dark_aurora,
    "wallpaper-dawn.svg": light_dawn,
    "dot.svg": '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><circle cx="8" cy="8" r="6" fill="#f2b84b"/></svg>',
    "bullet.svg": '<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14"><circle cx="7" cy="7" r="5" fill="#e6edf3"/></svg>',
    "entry.svg": '<svg xmlns="http://www.w3.org/2000/svg" width="380" height="48"><rect x="1.5" y="1.5" width="377" height="45" rx="12" fill="#141b22" fill-opacity="0.92" stroke="#f2b84b" stroke-opacity="0.7" stroke-width="2"/></svg>',
}
for n, c in svgs.items():
    (here / n).write_text(c)

def render(svg, png, w=None, h=None):
    cmd = ["rsvg-convert", str(here / svg), "-o", str(png)]
    if w: cmd += ["-w", str(w)]
    if h: cmd += ["-h", str(h)]
    subprocess.run(cmd, check=True)

bg = prof / "backgrounds/steady"
bg.mkdir(parents=True, exist_ok=True)
for old in bg.glob("*.png"): old.unlink()
render("wallpaper-steady.svg", bg / "steady-default.png")
render("wallpaper-aurora.svg", bg / "steady-aurora.png")
render("wallpaper-dawn.svg", bg / "steady-dawn.png")
ply = prof / "plymouth/themes/steady"
render("logo.svg", ply / "logo.png", 180, 180)
render("logo.svg", here / "logo-512.png")
for n in ("dot", "bullet", "entry"):
    render(n + ".svg", ply / (n + ".png"))
print("art done")
