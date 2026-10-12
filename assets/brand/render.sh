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
# The Windows resource icon `tauri-build` compiles into the Tauri example, which it reads only from an `.ico`.
ico=$(mktemp -d)
for size in 16 32 48 256; do
  rsvg-convert --width "$size" --height "$size" "$brand/icon-macos.svg" --output "$ico/$size.png"
done
convert "$ico/16.png" "$ico/32.png" "$ico/48.png" "$ico/256.png" "$brand/icon-macos.ico"
rm -r "$ico"
# The repository's social preview, uploaded in the GitHub settings.
rsvg-convert --width 1280 --height 640 "$brand/social.svg" --output "$brand/png/social.png"

# The Dioxus example's web app icons, written where `dx` serves them from.
public=$brand/../../examples/dioxus/public
mkdir -p "$public/icons"
for size in 192 512; do
  rsvg-convert --width "$size" --height "$size" "$brand/icon.svg" --output "$public/icons/icon-$size.png"
  rsvg-convert --width "$size" --height "$size" "$brand/icon-maskable.svg" --output "$public/icons/icon-maskable-$size.png"
done
# iOS reads the home screen icon from this link and rounds its corners itself.
rsvg-convert --width 180 --height 180 "$brand/icon.svg" --output "$public/icons/apple-touch-icon.png"

oxipng --quiet --opt max --strip safe "$brand"/png/*.png "$public"/icons/*.png
