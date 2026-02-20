#!/usr/bin/env python3
"""
Corkscrew Brand Kit Generator

Generates macOS-native icon with dark gradient background, specular edge
highlighting, and proper padding. Creates all sizes for Tauri (macOS, Windows, Linux).

The icon uses a dark charcoal-to-deep-gray gradient background that matches
the app's dark theme, with a subtle specular highlight along the top edge
for that native macOS dock feel.

Requirements: pip install Pillow numpy
"""

import math
import sys
from pathlib import Path

try:
    from PIL import Image, ImageDraw, ImageFilter
    import numpy as np
except ImportError as e:
    print(f"Missing dependency: {e}")
    print("Install with: pip install Pillow numpy")
    sys.exit(1)


# --- Color palette (matches app.css dark theme) ---
BG_TOP = (58, 58, 62)       # Lighter charcoal (top of gradient)
BG_BOTTOM = (28, 28, 31)    # Deep dark (bottom of gradient, --bg-base)
HIGHLIGHT_COLOR = (255, 255, 255)  # Specular highlight
EDGE_GLOW = (200, 140, 60)  # Warm amber edge glow (complements corkscrew orange)


def make_squircle_mask(size: int, radius_pct: float = 0.225) -> Image.Image:
    """Create a macOS-style continuous squircle (superellipse) mask.

    macOS Big Sur+ uses a specific superellipse shape for icons.
    radius_pct ~22.5% matches the macOS icon corner radius.
    """
    mask = Image.new("L", (size, size), 0)
    draw = ImageDraw.Draw(mask)
    radius = int(size * radius_pct)
    draw.rounded_rectangle([0, 0, size - 1, size - 1], radius=radius, fill=255)
    return mask


def make_gradient_background(size: int) -> Image.Image:
    """Create a dark vertical gradient background matching the app's theme."""
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    arr = np.array(img, dtype=np.float32)

    for y in range(size):
        t = y / max(size - 1, 1)
        # Slight ease-in curve for more natural gradient
        t = t * t * (3 - 2 * t)  # smoothstep
        r = BG_TOP[0] + (BG_BOTTOM[0] - BG_TOP[0]) * t
        g = BG_TOP[1] + (BG_BOTTOM[1] - BG_TOP[1]) * t
        b = BG_TOP[2] + (BG_BOTTOM[2] - BG_TOP[2]) * t
        arr[y, :, 0] = r
        arr[y, :, 1] = g
        arr[y, :, 2] = b
        arr[y, :, 3] = 255

    return Image.fromarray(arr.clip(0, 255).astype(np.uint8))


def add_specular_highlight(img: Image.Image) -> Image.Image:
    """Add a subtle specular highlight along the top edge, like native macOS icons."""
    size = img.width
    arr = np.array(img, dtype=np.float32)

    # Top specular band — bright highlight fading down
    highlight_height = int(size * 0.35)
    for y in range(highlight_height):
        t = 1.0 - (y / max(highlight_height - 1, 1))
        # Strong at top, fades quickly
        intensity = t * t * t * 0.18  # max 18% white overlay at very top
        for c in range(3):
            arr[y, :, c] = arr[y, :, c] + (255 - arr[y, :, c]) * intensity

    # Subtle radial highlight from top-center (simulates light source)
    cx, cy = size // 2, int(size * 0.15)
    max_radius = size * 0.55
    y_coords, x_coords = np.mgrid[0:size, 0:size]
    dist = np.sqrt((x_coords - cx) ** 2 + ((y_coords - cy) * 1.4) ** 2)
    radial = np.clip(1.0 - dist / max_radius, 0, 1)
    radial = radial ** 2.5 * 0.08  # Subtle — max 8% at center

    for c in range(3):
        arr[:, :, c] = arr[:, :, c] + (255 - arr[:, :, c]) * radial

    return Image.fromarray(arr.clip(0, 255).astype(np.uint8))


def add_edge_highlight(img: Image.Image, mask: Image.Image) -> Image.Image:
    """Add a subtle inner edge glow along the squircle border."""
    size = img.width
    arr = np.array(img, dtype=np.float32)
    mask_arr = np.array(mask, dtype=np.float32) / 255.0

    # Create inner edge by eroding the mask and subtracting
    erode_size = max(3, int(size * 0.015)) | 1
    eroded = mask.filter(ImageFilter.MinFilter(size=erode_size))
    eroded_arr = np.array(eroded, dtype=np.float32) / 255.0

    # Edge band: where mask is solid but eroded mask falls off
    edge = mask_arr - eroded_arr
    edge = np.clip(edge * 2.0, 0, 1)

    # Blur the edge for a soft glow
    edge_img = Image.fromarray((edge * 255).astype(np.uint8))
    blur_radius = max(1, size * 0.008)
    edge_img = edge_img.filter(ImageFilter.GaussianBlur(radius=blur_radius))
    edge = np.array(edge_img, dtype=np.float32) / 255.0

    # Apply as a subtle white inner highlight (stronger at top)
    for y in range(size):
        t = 1.0 - (y / max(size - 1, 1))
        intensity = 0.15 + t * 0.25  # 15% at bottom, 40% at top
        for c in range(3):
            arr[y, :, c] = arr[y, :, c] + (HIGHLIGHT_COLOR[c] - arr[y, :, c]) * edge[y, :] * intensity

    return Image.fromarray(arr.clip(0, 255).astype(np.uint8))


def add_shadow(img: Image.Image, mask: Image.Image) -> Image.Image:
    """Add a subtle drop shadow beneath the icon (visible at large sizes)."""
    size = img.width
    pad = int(size * 0.05)
    canvas_size = size + pad * 2

    # Create shadow from mask
    shadow = Image.new("RGBA", (canvas_size, canvas_size), (0, 0, 0, 0))
    shadow_offset = max(2, int(size * 0.01))
    shadow.paste(Image.new("L", (size, size), 40), (pad, pad + shadow_offset), mask)
    shadow = shadow.filter(ImageFilter.GaussianBlur(radius=max(2, size * 0.02)))

    # Paste icon on top
    shadow.paste(img, (pad, pad), mask)

    # Crop back to icon size with a bit of shadow visible
    crop_pad = max(1, pad // 3)
    result = shadow.crop((crop_pad, crop_pad, canvas_size - crop_pad, canvas_size - crop_pad))
    return result.resize((size, size), Image.Resampling.LANCZOS)


def build_macos_icon(transparent_artwork: Image.Image, size: int, full_bleed: bool = False) -> Image.Image:
    """Compose a macOS-style icon at the given size.

    Args:
        transparent_artwork: The corkscrew artwork with transparent background.
        size: Output icon size in pixels.
        full_bleed: If True, gradient fills entire square (for Tauri/OS icons where
                    macOS applies its own squircle mask). If False, bakes in the
                    squircle (for brand-kit/web/marketing use).

    Layers (bottom to top):
    1. Dark gradient background
    2. Specular highlight overlay
    3. Inner edge glow (squircle mode only)
    4. Corkscrew artwork (centered, ~80% of canvas)
    """
    # 1. Gradient background
    bg = make_gradient_background(size)

    # 2. Apply specular highlight to background
    bg = add_specular_highlight(bg)

    # Artwork fill: 62% of canvas — matches native macOS icon proportions
    # (e.g., Finder, Safari, Settings all have ~60-65% artwork fill)
    art_fill = 0.62

    if full_bleed:
        # Full square — macOS will apply its own squircle mask
        art_size = int(size * art_fill)
        art_pad = (size - art_size) // 2
        artwork = transparent_artwork.resize((art_size, art_size), Image.Resampling.LANCZOS)
        bg.paste(artwork, (art_pad, art_pad), artwork)
        return bg
    else:
        # Baked squircle for brand-kit / web use
        mask = make_squircle_mask(size)
        bg.putalpha(mask)
        bg = add_edge_highlight(bg, mask)

        art_size = int(size * art_fill)
        art_pad = (size - art_size) // 2
        artwork = transparent_artwork.resize((art_size, art_size), Image.Resampling.LANCZOS)
        bg.paste(artwork, (art_pad, art_pad), artwork)

        # Re-apply squircle mask to clip everything cleanly
        bg.putalpha(mask)

        return bg


def main():
    brand_dir = Path(__file__).parent
    project_dir = brand_dir.parent

    # Load the transparent artwork
    transparent_path = brand_dir / "corkscrew-transparent.png"
    if not transparent_path.exists():
        print(f"Transparent artwork not found at {transparent_path}")
        print("Run the background removal first or place corkscrew-transparent.png in brand-kit/")
        sys.exit(1)

    print(f"Loading transparent artwork: {transparent_path}")
    artwork = Image.open(transparent_path).convert("RGBA")
    print(f"  Size: {artwork.size[0]}x{artwork.size[1]}")

    # Generate the master icon at high resolution
    print("\nBuilding macOS-style icon (1024x1024)...")
    master = build_macos_icon(artwork, 1024)
    master_path = brand_dir / "corkscrew-icon-1024.png"
    master.save(master_path, "PNG", optimize=True)
    print(f"  Saved: {master_path.name} ({master_path.stat().st_size:,} bytes)")

    # Generate brand-kit sizes from master
    sizes = [512, 256, 128, 64, 48, 32, 24, 16]
    print("\nGenerating brand-kit icon sizes...")
    for size in sizes:
        # For small sizes, rebuild from scratch for sharpness
        if size >= 64:
            icon = build_macos_icon(artwork, size)
        else:
            # Very small: downscale from 256 for better quality
            icon = build_macos_icon(artwork, 256).resize((size, size), Image.Resampling.LANCZOS)
        out_path = brand_dir / f"corkscrew-icon-{size}.png"
        icon.save(out_path, "PNG", optimize=True)
        print(f"  {out_path.name} ({out_path.stat().st_size:,} bytes)")

    # --- Copy to static/ for in-app use ---
    static_dir = project_dir / "static"
    print("\nCopying to static/...")

    # 128px for sidebar/header
    icon_128 = build_macos_icon(artwork, 128)
    icon_128.save(static_dir / "corkscrew-icon.png", "PNG", optimize=True)
    print(f"  -> static/corkscrew-icon.png (128x128)")

    # 64px small variant
    icon_64 = build_macos_icon(artwork, 64)
    icon_64.save(static_dir / "corkscrew-icon-sm.png", "PNG", optimize=True)
    print(f"  -> static/corkscrew-icon-sm.png (64x64)")

    # --- Generate Tauri app icons ---
    # macOS Big Sur+: The system applies its own squircle mask automatically.
    # So Tauri icons must be full-bleed squares (gradient fills entire canvas).
    # If we bake in rounded corners, the OS mask clips them and the icon
    # appears smaller than other dock icons.
    icons_dir = project_dir / "src-tauri" / "icons"
    print("\nGenerating Tauri app icons (full-bleed for OS masking)...")

    # icon.png — 512x512 main icon (full-bleed)
    icon_512 = build_macos_icon(artwork, 512, full_bleed=True)
    icon_512.save(icons_dir / "icon.png", "PNG", optimize=True)
    print(f"  icon.png (512x512, full-bleed)")

    # 128x128 and 128x128@2x (256x256) — full-bleed
    icon_128_tauri = build_macos_icon(artwork, 128, full_bleed=True)
    icon_128_tauri.save(icons_dir / "128x128.png", "PNG", optimize=True)
    print(f"  128x128.png (full-bleed)")

    icon_256 = build_macos_icon(artwork, 256, full_bleed=True)
    icon_256.save(icons_dir / "128x128@2x.png", "PNG", optimize=True)
    print(f"  128x128@2x.png (256x256, full-bleed)")

    # 32x32 — full-bleed
    icon_32 = build_macos_icon(artwork, 32, full_bleed=True)
    icon_32.save(icons_dir / "32x32.png", "PNG", optimize=True)
    print(f"  32x32.png (full-bleed)")

    # Windows Square logos — full-bleed (Windows tiles are edge-to-edge)
    win_sizes = [
        ("Square30x30Logo.png", 30),
        ("Square44x44Logo.png", 44),
        ("Square71x71Logo.png", 71),
        ("Square89x89Logo.png", 89),
        ("Square107x107Logo.png", 107),
        ("Square142x142Logo.png", 142),
        ("Square150x150Logo.png", 150),
        ("Square284x284Logo.png", 284),
        ("Square310x310Logo.png", 310),
        ("StoreLogo.png", 50),
    ]
    print("  Windows logos (full-bleed)...")
    for name, size in win_sizes:
        if size >= 64:
            icon = build_macos_icon(artwork, size, full_bleed=True)
        else:
            icon = build_macos_icon(artwork, 256, full_bleed=True).resize(
                (size, size), Image.Resampling.LANCZOS)
        icon.save(icons_dir / name, "PNG", optimize=True)
    print(f"  {len(win_sizes)} Windows icons generated")

    # icon.ico — 256x256 (full-bleed)
    icon_ico = build_macos_icon(artwork, 256, full_bleed=True)
    icon_ico.save(icons_dir / "icon.ico", format="ICO", sizes=[(256, 256)])
    print(f"  icon.ico (256x256, full-bleed)")

    # icon.icns — macOS native format
    # Tauri generates this from icon.png during build, but we can also
    # create it manually for best quality
    try:
        import subprocess
        # Use iconutil on macOS to create .icns from iconset
        iconset_dir = icons_dir / "icon.iconset"
        iconset_dir.mkdir(exist_ok=True)

        icns_sizes = [
            ("icon_16x16.png", 16),
            ("icon_16x16@2x.png", 32),
            ("icon_32x32.png", 32),
            ("icon_32x32@2x.png", 64),
            ("icon_128x128.png", 128),
            ("icon_128x128@2x.png", 256),
            ("icon_256x256.png", 256),
            ("icon_256x256@2x.png", 512),
            ("icon_512x512.png", 512),
            ("icon_512x512@2x.png", 1024),
        ]

        for name, size in icns_sizes:
            icon = build_macos_icon(artwork, size, full_bleed=True) if size >= 64 else \
                   build_macos_icon(artwork, 256, full_bleed=True).resize(
                       (size, size), Image.Resampling.LANCZOS)
            icon.save(iconset_dir / name, "PNG", optimize=True)

        result = subprocess.run(
            ["iconutil", "-c", "icns", str(iconset_dir), "-o", str(icons_dir / "icon.icns")],
            capture_output=True, text=True
        )
        if result.returncode == 0:
            print(f"  icon.icns (native macOS, {(icons_dir / 'icon.icns').stat().st_size:,} bytes)")
        else:
            print(f"  icon.icns generation failed: {result.stderr}")

        # Clean up iconset
        import shutil
        shutil.rmtree(iconset_dir, ignore_errors=True)

    except (FileNotFoundError, ImportError):
        print("  icon.icns — skipped (iconutil not available, Tauri will generate from icon.png)")

    print(f"\nDone! All icons generated.")
    print(f"  Brand kit: {brand_dir}")
    print(f"  Static:    {static_dir}")
    print(f"  Tauri:     {icons_dir}")


if __name__ == "__main__":
    main()
