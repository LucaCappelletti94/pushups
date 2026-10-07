"""Derives the brand's SVG variants from the mark in logo.svg, writing them beside it."""

import re
from pathlib import Path

brand = Path(__file__).parent
source = (brand / "logo.svg").read_text()
gradient = re.search(r"<linearGradient.*?</linearGradient>", source, re.S).group(0)
mark = re.search(r'<path fill="url\(#g\)" d=".*?"/>', source, re.S).group(0)

NAVY = "#0e1824"
TEXT = "#f5f7fa"
MUTED = "#9aa6b8"
FONT = "Inter, Helvetica, Arial, sans-serif"
# The mark's bounding box in logo.svg's 800 by 800 space.
X0, Y0, W, H = 108, 215, 485, 375


def placed(size, width_share, center=None):
    """The mark scaled to `width_share` of `size`, centred on `center` or on the square's middle."""
    cx, cy = center or (size / 2, size / 2)
    scale = size * width_share / W
    dx = cx - (X0 + W / 2) * scale
    dy = cy - (Y0 + H / 2) * scale
    return f'<g transform="translate({dx:.2f} {dy:.2f}) scale({scale:.4f})">\n    {gradient}\n    {mark}\n  </g>'


def svg(name, width, height, body, view_box=None):
    view_box = view_box or f"0 0 {width} {height}"
    (brand / name).write_text(
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="{view_box}">\n  {body}\n</svg>\n'
    )


pad = 12
svg("mark.svg", W + 2 * pad, H + 2 * pad, f"{gradient}\n  {mark}", f"{X0 - pad} {Y0 - pad} {W + 2 * pad} {H + 2 * pad}")
svg("icon.svg", 1024, 1024, f'<rect width="1024" height="1024" fill="{NAVY}"/>\n  {placed(1024, 0.66)}')
# Apple's macOS icon grid, an 824 square with a 185 radius on a 1024 canvas.
svg("icon-macos.svg", 1024, 1024, f'<rect x="100" y="100" width="824" height="824" rx="185" fill="{NAVY}"/>\n  {placed(1024, 0.48)}')
# A maskable web icon, full bleed, with the mark inside the 80% circle Android crops to.
svg("icon-maskable.svg", 1024, 1024, f'<rect width="1024" height="1024" fill="{NAVY}"/>\n  {placed(1024, 0.56)}')
svg("favicon.svg", 64, 64, f'<rect width="64" height="64" rx="14" fill="{NAVY}"/>\n  {placed(64, 0.78)}')
svg(
    "social.svg",
    1280,
    640,
    f'<rect width="1280" height="640" fill="{NAVY}"/>\n  {placed(1280, 0.36, center=(340, 320))}\n'
    f'  <text x="610" y="330" fill="{TEXT}" font-family="{FONT}" font-size="120" font-weight="700">pushups</text>\n'
    f'  <text x="616" y="404" fill="{MUTED}" font-family="{FONT}" font-size="40">Push notifications for Rust apps</text>',
)
