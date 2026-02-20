#!/usr/bin/env python3
"""
Corkscrew Brand Kit Generator

Generates a proper macOS-native icon following Apple HIG:
- 1024x1024 canvas with ~100px transparent padding per side
- ~824x824 squircle body with dark gradient background
- Specular highlights and edge glow
- Corkscrew artwork fills ~75% of the squircle body
- Separate .icns for macOS (baked squircle) and icon.png for Linux (full-bleed)

macOS does NOT auto-apply a squircle mask (until macOS 26 Tahoe).
The squircle MUST be baked into the .icns file.

Requirements: pip install Pillow numpy
"""

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
HIGHLIGHT_COLOR = (255, 255, 255)

# Apple HIG icon proportions (for 1024x1024 canvas):
# - Squircle body: ~824x824 (80.5% of canvas)
# - Transparent padding: ~100px per side (9.75%)
# - Corner radius: ~185px on 824px body (~22.5%)
BODY_PCT = 0.826       # squircle body as fraction of canvas (matches Claude, Discord)
CORNER_RADIUS_PCT = 0.225  # corner radius as fraction of body size
ARTWORK_FILL = 0.65    # artwork fills 65% of squircle body


def make_squircle_mask(body_size: int) -> Image.Image:
    """Create a macOS-style rounded rectangle mask at the given body size."""
    mask = Image.new("L", (body_size, body_size), 0)
    draw = ImageDraw.Draw(mask)
    radius = int(body_size * CORNER_RADIUS_PCT)
    draw.rounded_rectangle([0, 0, body_size - 1, body_size - 1], radius=radius, fill=255)
    return mask


def make_gradient(size: int) -> Image.Image:
    """Create a dark vertical gradient background."""
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    arr = np.array(img, dtype=np.float32)

    for y in range(size):
        t = y / max(size - 1, 1)
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
    """Add a subtle specular highlight along the top edge."""
    size = img.width
    arr = np.array(img, dtype=np.float32)

    # Top specular band
    highlight_height = int(size * 0.35)
    for y in range(highlight_height):
        t = 1.0 - (y / max(highlight_height - 1, 1))
        intensity = t * t * t * 0.18
        for c in range(3):
            arr[y, :, c] = arr[y, :, c] + (255 - arr[y, :, c]) * intensity

    # Radial highlight from top-center
    cx, cy = size // 2, int(size * 0.15)
    max_radius = size * 0.55
    y_coords, x_coords = np.mgrid[0:size, 0:size]
    dist = np.sqrt((x_coords - cx) ** 2 + ((y_coords - cy) * 1.4) ** 2)
    radial = np.clip(1.0 - dist / max_radius, 0, 1)
    radial = radial ** 2.5 * 0.08

    for c in range(3):
        arr[:, :, c] = arr[:, :, c] + (255 - arr[:, :, c]) * radial

    return Image.fromarray(arr.clip(0, 255).astype(np.uint8))


def add_edge_highlight(img: Image.Image, mask: Image.Image) -> Image.Image:
    """Add a subtle inner edge glow along the squircle border."""
    size = img.width
    arr = np.array(img, dtype=np.float32)
    mask_arr = np.array(mask, dtype=np.float32) / 255.0

    erode_size = max(3, int(size * 0.015)) | 1
    eroded = mask.filter(ImageFilter.MinFilter(size=erode_size))
    eroded_arr = np.array(eroded, dtype=np.float32) / 255.0

    edge = np.clip((mask_arr - eroded_arr) * 2.0, 0, 1)

    edge_img = Image.fromarray((edge * 255).astype(np.uint8))
    blur_radius = max(1, size * 0.008)
    edge_img = edge_img.filter(ImageFilter.GaussianBlur(radius=blur_radius))
    edge = np.array(edge_img, dtype=np.float32) / 255.0

    for y in range(size):
        t = 1.0 - (y / max(size - 1, 1))
        intensity = 0.15 + t * 0.25
        for c in range(3):
            arr[y, :, c] = arr[y, :, c] + (HIGHLIGHT_COLOR[c] - arr[y, :, c]) * edge[y, :] * intensity

    return Image.fromarray(arr.clip(0, 255).astype(np.uint8))


def build_macos_icon(transparent_artwork: Image.Image, canvas_size: int) -> Image.Image:
    """Build a proper macOS icon with baked squircle and transparent padding.

    Following Apple HIG:
    - Canvas: full size (e.g. 1024x1024)
    - Squircle body: ~80.5% of canvas, centered
    - Artwork: ~75% of squircle body, centered within it
    - Transparent padding around the squircle
    """
    # Calculate body size and padding
    body_size = int(canvas_size * BODY_PCT)
    pad = (canvas_size - body_size) // 2

    # 1. Create gradient background at body size
    bg = make_gradient(body_size)

    # 2. Add specular highlight
    bg = add_specular_highlight(bg)

    # 3. Create squircle mask at body size
    mask = make_squircle_mask(body_size)

    # 4. Apply mask
    bg.putalpha(mask)

    # 5. Add edge highlight
    bg = add_edge_highlight(bg, mask)

    # 6. Composite artwork (75% of squircle body)
    art_size = int(body_size * ARTWORK_FILL)
    art_pad = (body_size - art_size) // 2
    artwork = transparent_artwork.resize((art_size, art_size), Image.Resampling.LANCZOS)
    bg.paste(artwork, (art_pad, art_pad), artwork)

    # 7. Re-apply mask for clean edges
    bg.putalpha(mask)

    # 8. Place on transparent canvas with padding
    canvas = Image.new("RGBA", (canvas_size, canvas_size), (0, 0, 0, 0))
    canvas.paste(bg, (pad, pad), bg)

    return canvas


def build_linux_icon(transparent_artwork: Image.Image, size: int) -> Image.Image:
    """Build a full-bleed icon for Linux (no squircle, no padding).

    Linux desktop environments don't apply icon masks, so fill the
    entire square with the gradient and artwork.
    """
    bg = make_gradient(size)
    bg = add_specular_highlight(bg)

    # Artwork at 75% of full canvas
    art_size = int(size * ARTWORK_FILL)
    art_pad = (size - art_size) // 2
    artwork = transparent_artwork.resize((art_size, art_size), Image.Resampling.LANCZOS)
    bg.paste(artwork, (art_pad, art_pad), artwork)

    return bg


def build_brandkit_icon(transparent_artwork: Image.Image, size: int) -> Image.Image:
    """Build a brand-kit icon (squircle fills the canvas, no extra padding).

    For web/marketing use where we want the squircle to fill the image.
    """
    bg = make_gradient(size)
    bg = add_specular_highlight(bg)
    mask = make_squircle_mask(size)
    bg.putalpha(mask)
    bg = add_edge_highlight(bg, mask)

    art_size = int(size * ARTWORK_FILL)
    art_pad = (size - art_size) // 2
    artwork = transparent_artwork.resize((art_size, art_size), Image.Resampling.LANCZOS)
    bg.paste(artwork, (art_pad, art_pad), artwork)
    bg.putalpha(mask)

    return bg


def main():
    brand_dir = Path(__file__).parent
    project_dir = brand_dir.parent

    transparent_path = brand_dir / "corkscrew-transparent.png"
    if not transparent_path.exists():
        print(f"Transparent artwork not found at {transparent_path}")
        print("Run the background removal first or place corkscrew-transparent.png in brand-kit/")
        sys.exit(1)

    print(f"Loading transparent artwork: {transparent_path}")
    artwork = Image.open(transparent_path).convert("RGBA")
    print(f"  Size: {artwork.size[0]}x{artwork.size[1]}")

    # --- Brand-kit icons (squircle fills canvas, for web/marketing) ---
    print("\nGenerating brand-kit icons (squircle, no outer padding)...")
    for size in [1024, 512, 256, 128, 64, 48, 32, 24, 16]:
        if size >= 64:
            icon = build_brandkit_icon(artwork, size)
        else:
            icon = build_brandkit_icon(artwork, 256).resize(
                (size, size), Image.Resampling.LANCZOS)
        out_path = brand_dir / f"corkscrew-icon-{size}.png"
        icon.save(out_path, "PNG", optimize=True)
        print(f"  {out_path.name} ({out_path.stat().st_size:,} bytes)")

    # --- Static icons for in-app use (squircle style) ---
    static_dir = project_dir / "static"
    print("\nCopying to static/...")

    icon_128 = build_brandkit_icon(artwork, 128)
    icon_128.save(static_dir / "corkscrew-icon.png", "PNG", optimize=True)
    print(f"  -> static/corkscrew-icon.png (128x128)")

    icon_64 = build_brandkit_icon(artwork, 64)
    icon_64.save(static_dir / "corkscrew-icon-sm.png", "PNG", optimize=True)
    print(f"  -> static/corkscrew-icon-sm.png (64x64)")

    # --- Tauri app icons ---
    icons_dir = project_dir / "src-tauri" / "icons"
    print("\nGenerating Tauri app icons...")

    # icon.png — Used by Linux and as fallback. Full-bleed (no squircle).
    icon_512 = build_linux_icon(artwork, 512)
    icon_512.save(icons_dir / "icon.png", "PNG", optimize=True)
    print(f"  icon.png (512x512, full-bleed for Linux)")

    # Standard PNG sizes (full-bleed for Linux)
    icon_128_tauri = build_linux_icon(artwork, 128)
    icon_128_tauri.save(icons_dir / "128x128.png", "PNG", optimize=True)
    print(f"  128x128.png")

    icon_256 = build_linux_icon(artwork, 256)
    icon_256.save(icons_dir / "128x128@2x.png", "PNG", optimize=True)
    print(f"  128x128@2x.png (256x256)")

    icon_32 = build_linux_icon(artwork, 32)
    icon_32.save(icons_dir / "32x32.png", "PNG", optimize=True)
    print(f"  32x32.png")

    # Windows Square logos (full-bleed)
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
    print("  Windows logos...")
    for name, size in win_sizes:
        if size >= 64:
            icon = build_linux_icon(artwork, size)
        else:
            icon = build_linux_icon(artwork, 256).resize(
                (size, size), Image.Resampling.LANCZOS)
        icon.save(icons_dir / name, "PNG", optimize=True)
    print(f"  {len(win_sizes)} Windows icons generated")

    # icon.ico (full-bleed for Windows)
    icon_ico = build_linux_icon(artwork, 256)
    icon_ico.save(icons_dir / "icon.ico", format="ICO", sizes=[(256, 256)])
    print(f"  icon.ico (256x256)")

    # --- icon.icns — macOS native format ---
    # This is the critical one. macOS does NOT auto-apply the squircle mask.
    # We must bake the squircle into the .icns with proper padding.
    # Apple HIG: ~100px padding on 1024x1024, squircle body ~824x824.
    print("\n  Generating icon.icns (macOS, baked squircle with padding)...")
    try:
        import subprocess
        import shutil

        iconset_dir = icons_dir / "icon.iconset"
        iconset_dir.mkdir(exist_ok=True)

        # Each entry in the iconset needs the macOS-style icon
        # (baked squircle + transparent padding)
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

        for name, canvas_size in icns_sizes:
            if canvas_size >= 64:
                icon = build_macos_icon(artwork, canvas_size)
            else:
                # For very small sizes, downscale from a larger render
                icon = build_macos_icon(artwork, 256).resize(
                    (canvas_size, canvas_size), Image.Resampling.LANCZOS)
            icon.save(iconset_dir / name, "PNG", optimize=True)

        result = subprocess.run(
            ["iconutil", "-c", "icns", str(iconset_dir), "-o", str(icons_dir / "icon.icns")],
            capture_output=True, text=True
        )
        if result.returncode == 0:
            print(f"  icon.icns ({(icons_dir / 'icon.icns').stat().st_size:,} bytes)")
        else:
            print(f"  icon.icns generation failed: {result.stderr}")

        shutil.rmtree(iconset_dir, ignore_errors=True)

    except (FileNotFoundError, ImportError):
        print("  icon.icns — skipped (iconutil not available)")

    print(f"\nDone! All icons generated.")
    print(f"  Brand kit: {brand_dir}")
    print(f"  Static:    {static_dir}")
    print(f"  Tauri:     {icons_dir}")


if __name__ == "__main__":
    main()
