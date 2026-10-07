#!/usr/bin/env bash
# Derives the SVG variants from logo.svg, renders the rasters with rsvg-convert and shrinks each losslessly with oxipng.
set -euo pipefail
brand=$(cd "$(dirname "$0")" && pwd)
python3 "$brand/variants.py"
mkdir -p "$brand/png"

# The app icon sizes `dx bundle` turns into .icns, .ico and Linux icons.
for size in 32 128 256 512 1024; do
  rsvg-convert --width "$size" --height "$size" "$brand/icon-macos.svg" --output "$brand/png/icon-macos-$size.png"
done
# The repository's social preview, uploaded in the GitHub settings.
rsvg-convert --width 1280 --height 640 "$brand/social.svg" --output "$brand/png/social.png"

oxipng --quiet --opt max --strip safe "$brand"/png/*.png
