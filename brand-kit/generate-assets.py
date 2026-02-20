#!/usr/bin/env python3
"""
Corkscrew Brand Kit Generator

Generates transparent PNG icon variants from the high-res source icon.
Uses numpy for fast processing at full resolution (5695x5695).
Removes gray background, defrings edges, and downscales to all needed sizes.

Requirements: pip install Pillow numpy
"""

import sys
from pathlib import Path

try:
    from PIL import Image, ImageFilter
    import numpy as np
except ImportError as e:
    print(f"Missing dependency: {e}")
    print("Install with: pip install Pillow numpy")
    sys.exit(1)


def remove_background(img: Image.Image) -> Image.Image:
    """Remove gray background using warmth/saturation masking with aggressive cleanup."""
    img = img.convert("RGBA")
    arr = np.array(img, dtype=np.float32)
    h, w = arr.shape[:2]
    r, g, b = arr[:, :, 0], arr[:, :, 1], arr[:, :, 2]

    # Sample background color for defringe later
    sample = max(50, min(w, h) // 20)
    bg_r = np.mean([arr[:sample, :sample, 0].mean(), arr[:sample, -sample:, 0].mean()])
    bg_g = np.mean([arr[:sample, :sample, 1].mean(), arr[:sample, -sample:, 1].mean()])
    bg_b = np.mean([arr[:sample, :sample, 2].mean(), arr[:sample, -sample:, 2].mean()])
    print(f"  Sampled background: RGB({bg_r:.0f}, {bg_g:.0f}, {bg_b:.0f})")

    # Core discriminator: saturation + warmth (R-B)
    max_c = np.maximum(np.maximum(r, g), b)
    min_c = np.minimum(np.minimum(r, g), b)
    chroma = max_c - min_c
    saturation = np.divide(chroma, np.maximum(max_c, 1.0))
    warmth = np.clip((r - b) / 255.0, 0.0, None)

    # Foreground score: high saturation AND warm
    score = np.clip(saturation * 3.0, 0.0, 1.0) * np.clip(warmth * 4.0, 0.0, 1.0)

    # Kill anything with very low saturation (gray background)
    score[saturation < 0.10] = 0.0

    # Catch darker warm-orange areas (shadows on the corkscrew)
    warm_dark = (warmth > 0.06) & (saturation > 0.15)
    score[warm_dark] = np.maximum(score[warm_dark], np.clip(saturation[warm_dark] * 2.5, 0.0, 1.0))

    # Convert to 8-bit mask
    mask_arr = (score * 255.0).clip(0, 255).astype(np.uint8)
    mask = Image.fromarray(mask_arr)

    # Smooth proportionally to image size
    scale = max(w, h) / 1024.0
    mask = mask.filter(ImageFilter.GaussianBlur(radius=max(1.0, 1.0 * scale)))

    # Hard threshold: push toward fully transparent or fully opaque
    mask_arr = np.array(mask, dtype=np.float32)
    result_mask = np.zeros_like(mask_arr)
    result_mask[mask_arr < 35] = 0.0
    mid = (mask_arr >= 35) & (mask_arr < 90)
    result_mask[mid] = ((mask_arr[mid] - 35) / 55.0) * 255.0
    result_mask[mask_arr >= 90] = 255.0
    mask = Image.fromarray(result_mask.clip(0, 255).astype(np.uint8))

    # Edge smooth
    mask = mask.filter(ImageFilter.GaussianBlur(radius=max(0.5, 0.5 * scale)))

    # Two-pass erosion to aggressively eat gray fringe
    erode_size = max(3, int(3 * scale)) | 1  # must be odd
    eroded = mask.filter(ImageFilter.MinFilter(size=erode_size))
    eroded = eroded.filter(ImageFilter.MinFilter(size=3))

    # Use eroded values for all non-interior pixels
    mask_arr = np.array(mask, dtype=np.float32)
    eroded_arr = np.array(eroded, dtype=np.float32)
    mask_arr[mask_arr < 250] = np.minimum(mask_arr[mask_arr < 250], eroded_arr[mask_arr < 250])
    mask = Image.fromarray(mask_arr.clip(0, 255).astype(np.uint8))

    # Final gentle smooth
    mask = mask.filter(ImageFilter.GaussianBlur(radius=max(0.3, 0.3 * scale)))

    # Apply mask
    result = img.copy()
    result.putalpha(mask)

    # Defringe: un-premultiply background contamination from semi-transparent edges
    result_arr = np.array(result, dtype=np.float32)
    alpha_arr = np.array(mask, dtype=np.float32)

    edge = (alpha_arr > 0) & (alpha_arr < 240)
    if np.any(edge):
        er = result_arr[:, :, 0][edge]
        eg = result_arr[:, :, 1][edge]
        eb = result_arr[:, :, 2][edge]
        ea = alpha_arr[edge]

        bg_blend = 1.0 - (ea / 255.0)
        a_safe = np.maximum(ea / 255.0, 0.05)

        # Remove background contamination: pixel = fg*a + bg*(1-a), so fg = (pixel - bg*(1-a))/a
        er_clean = np.clip((er - bg_r * bg_blend) / a_safe, 0, 255)
        eg_clean = np.clip((eg - bg_g * bg_blend) / a_safe, 0, 255)
        eb_clean = np.clip((eb - bg_b * bg_blend) / a_safe, 0, 255)

        # Blend toward cleaned color based on contamination level
        blend = np.clip(bg_blend * 1.2, 0, 1)
        result_arr[:, :, 0][edge] = er * (1 - blend) + er_clean * blend
        result_arr[:, :, 1][edge] = eg * (1 - blend) + eg_clean * blend
        result_arr[:, :, 2][edge] = eb * (1 - blend) + eb_clean * blend

    return Image.fromarray(result_arr.clip(0, 255).astype(np.uint8))


def trim_transparent(img: Image.Image, padding: int = 8) -> Image.Image:
    """Crop to content bounding box with padding."""
    bbox = img.getbbox()
    if bbox is None:
        return img
    x1, y1, x2, y2 = bbox
    x1 = max(0, x1 - padding)
    y1 = max(0, y1 - padding)
    x2 = min(img.width, x2 + padding)
    y2 = min(img.height, y2 + padding)
    return img.crop((x1, y1, x2, y2))


def make_square(img: Image.Image) -> Image.Image:
    """Center image on a square transparent canvas."""
    w, h = img.size
    size = max(w, h)
    result = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    result.paste(img, ((size - w) // 2, (size - h) // 2), img)
    return result


def main():
    brand_dir = Path(__file__).parent
    project_dir = brand_dir.parent

    # Use the high-res source in brand-kit/
    source_icon = brand_dir / "source-highres.png"
    if not source_icon.exists():
        # Fallback to Tauri icon
        source_icon = project_dir / "src-tauri" / "icons" / "icon.png"

    if not source_icon.exists():
        print(f"Source icon not found. Place source-highres.png in brand-kit/.")
        sys.exit(1)

    print(f"Loading source: {source_icon}")
    source = Image.open(source_icon)
    print(f"  Size: {source.size[0]}x{source.size[1]}, Mode: {source.mode}")

    print("\nRemoving background (numpy-accelerated)...")
    transparent = remove_background(source)

    print("Trimming and centering...")
    # Scale padding proportionally to image size
    pad = max(16, source.size[0] // 100)
    trimmed = trim_transparent(transparent, padding=pad)
    squared = make_square(trimmed)

    full_path = brand_dir / "corkscrew-transparent.png"
    squared.save(full_path, "PNG", optimize=True)
    print(f"  Full transparent: {full_path.name} ({squared.size[0]}x{squared.size[1]})")

    # Generate standard icon sizes
    sizes = [1024, 512, 256, 128, 64, 48, 32, 24, 16]
    print("\nGenerating icon sizes...")
    for size in sizes:
        resized = squared.resize((size, size), Image.Resampling.LANCZOS)
        out_path = brand_dir / f"corkscrew-icon-{size}.png"
        resized.save(out_path, "PNG", optimize=True)
        print(f"  {out_path.name} ({out_path.stat().st_size:,} bytes)")

    # Copy to static/ for app use
    static_dir = project_dir / "static"

    # 128px for sidebar (displayed at 28px, crisp on 3x+ retina)
    icon_128 = squared.resize((128, 128), Image.Resampling.LANCZOS)
    icon_128.save(static_dir / "corkscrew-icon.png", "PNG", optimize=True)
    print(f"\n  -> static/corkscrew-icon.png (128x128)")

    # 64px small variant
    icon_64 = squared.resize((64, 64), Image.Resampling.LANCZOS)
    icon_64.save(static_dir / "corkscrew-icon-sm.png", "PNG", optimize=True)
    print(f"  -> static/corkscrew-icon-sm.png (64x64)")

    # --- Generate Tauri app icons with proper macOS padding ---
    # macOS HIG: icon artwork should fill ~80% of canvas (10% padding per side)
    # This prevents the icon from appearing oversized vs other dock icons
    icons_dir = project_dir / "src-tauri" / "icons"
    print("\nGenerating Tauri app icons (with macOS padding)...")

    def make_padded_icon(source_img: Image.Image, target_size: int, padding_pct: float = 0.10) -> Image.Image:
        """Create an icon with macOS-appropriate padding around the artwork."""
        pad = int(target_size * padding_pct)
        content_size = target_size - (2 * pad)
        content = source_img.resize((content_size, content_size), Image.Resampling.LANCZOS)
        canvas = Image.new("RGBA", (target_size, target_size), (0, 0, 0, 0))
        canvas.paste(content, (pad, pad), content)
        return canvas

    # icon.png — 512x512 main icon (used by Tauri for .icns generation)
    padded_512 = make_padded_icon(squared, 512)
    padded_512.save(icons_dir / "icon.png", "PNG", optimize=True)
    print(f"  icon.png (512x512, 80% fill)")

    # 128x128 and 128x128@2x (256x256)
    padded_128 = make_padded_icon(squared, 128)
    padded_128.save(icons_dir / "128x128.png", "PNG", optimize=True)
    print(f"  128x128.png")

    padded_256 = make_padded_icon(squared, 256)
    padded_256.save(icons_dir / "128x128@2x.png", "PNG", optimize=True)
    print(f"  128x128@2x.png (256x256)")

    # 32x32
    padded_32 = make_padded_icon(squared, 32)
    padded_32.save(icons_dir / "32x32.png", "PNG", optimize=True)
    print(f"  32x32.png")

    # Windows Square logos (no extra padding — Windows tiles are edge-to-edge)
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
    for name, size in win_sizes:
        resized = squared.resize((size, size), Image.Resampling.LANCZOS)
        resized.save(icons_dir / name, "PNG", optimize=True)

    # icon.ico — 256x256 with padding
    padded_ico = make_padded_icon(squared, 256)
    padded_ico.save(icons_dir / "icon.ico", format="ICO", sizes=[(256, 256)])
    print(f"  icon.ico (256x256)")

    print(f"\nDone! Brand kit at: {brand_dir}")


if __name__ == "__main__":
    main()
