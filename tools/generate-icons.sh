#!/usr/bin/env bash
# generate-icons.sh — Convert a large PNG into all Tauri icon sizes (RGBA)
# Uses Python/Pillow for resizing (ensures RGBA), macOS iconutil for .icns
# Usage: ./tools/generate-icons.sh <source.png> [output-dir]
#
# Requires: Python 3 with Pillow. If not installed system-wide, the script
# will create a temporary venv and install it automatically.

set -euo pipefail

SOURCE="${1:?Usage: $0 <source.png> [output-dir]}"
OUTDIR="${2:-src-tauri/icons}"

if [ ! -f "$SOURCE" ]; then
  echo "Error: Source file '$SOURCE' not found"
  exit 1
fi

mkdir -p "$OUTDIR"

# --- Find or bootstrap a Python with Pillow ---
find_python() {
  # Check if system python has Pillow
  if python3 -c "from PIL import Image" 2>/dev/null; then
    echo "python3"
    return
  fi
  # Check for a cached venv
  local venv="/tmp/corkscrew-iconvenv"
  if [ -x "$venv/bin/python3" ] && "$venv/bin/python3" -c "from PIL import Image" 2>/dev/null; then
    echo "$venv/bin/python3"
    return
  fi
  # Create venv and install Pillow
  echo "  Installing Pillow in temporary venv..." >&2
  python3 -m venv "$venv"
  "$venv/bin/pip" install -q Pillow >/dev/null 2>&1
  echo "$venv/bin/python3"
}

PYTHON=$(find_python)
echo "Using Python: $PYTHON"

# --- Do all the work in a single Python invocation ---
"$PYTHON" - "$SOURCE" "$OUTDIR" << 'PYEOF'
import sys, os, struct
from PIL import Image

source_path = sys.argv[1]
out_dir = sys.argv[2]

img = Image.open(source_path).convert("RGBA")
w, h = img.size
print(f"Source: {w}x{h} (converted to RGBA)")
if w != h:
    print(f"Warning: Source is not square ({w}x{h}). Results may be distorted.")

def resize_save(size, dest):
    """Resize to size x size with high-quality Lanczos and save as RGBA PNG."""
    resized = img.resize((size, size), Image.LANCZOS)
    resized.save(dest, format="PNG")
    print(f"  {os.path.basename(dest)}: {size}x{size}")
    return dest

print()
print("=== Generating Tauri icons ===")

# --- Standard Tauri icons ---
print("Standard icons:")
resize_save(32,  os.path.join(out_dir, "32x32.png"))
resize_save(128, os.path.join(out_dir, "128x128.png"))
resize_save(256, os.path.join(out_dir, "128x128@2x.png"))
resize_save(512, os.path.join(out_dir, "icon.png"))

# --- Windows Store logos ---
print("Windows Store logos:")
for size, name in [
    (30, "Square30x30Logo.png"),   (44, "Square44x44Logo.png"),
    (71, "Square71x71Logo.png"),   (89, "Square89x89Logo.png"),
    (107, "Square107x107Logo.png"), (142, "Square142x142Logo.png"),
    (150, "Square150x150Logo.png"), (284, "Square284x284Logo.png"),
    (310, "Square310x310Logo.png"), (50, "StoreLogo.png"),
]:
    resize_save(size, os.path.join(out_dir, name))

# --- macOS .iconset for iconutil ---
print("macOS .iconset:")
import tempfile, subprocess, shutil

iconset_dir = tempfile.mkdtemp(suffix=".iconset")
for size, name in [
    (16,   "icon_16x16.png"),      (32,   "icon_16x16@2x.png"),
    (32,   "icon_32x32.png"),      (64,   "icon_32x32@2x.png"),
    (128,  "icon_128x128.png"),    (256,  "icon_128x128@2x.png"),
    (256,  "icon_256x256.png"),    (512,  "icon_256x256@2x.png"),
    (512,  "icon_512x512.png"),    (1024, "icon_512x512@2x.png"),
]:
    resize_save(size, os.path.join(iconset_dir, name))

icns_path = os.path.join(out_dir, "icon.icns")
subprocess.run(["iconutil", "-c", "icns", iconset_dir, "-o", icns_path], check=True)
icns_size = os.path.getsize(icns_path)
print(f"  icon.icns: {icns_size:,} bytes")
shutil.rmtree(iconset_dir)

# --- Windows .ico with embedded RGBA PNGs ---
print("Windows .ico:")
ico_sizes = [16, 32, 48, 64, 128, 256]
ico_pngs = []
for s in ico_sizes:
    buf = __import__("io").BytesIO()
    img.resize((s, s), Image.LANCZOS).save(buf, format="PNG")
    ico_pngs.append(buf.getvalue())

# Build ICO file: header + directory + PNG data
header = struct.pack('<HHH', 0, 1, len(ico_pngs))
dir_offset = 6 + len(ico_pngs) * 16
entries = b''
data_block = b''
for i, data in enumerate(ico_pngs):
    s = ico_sizes[i]
    dim = 0 if s >= 256 else s
    entries += struct.pack('<BBBBHHIH',
        dim, dim, 0, 0, 1, 32, len(data), dir_offset + len(data_block))
    data_block += data

ico_path = os.path.join(out_dir, "icon.ico")
with open(ico_path, 'wb') as f:
    f.write(header + entries + data_block)
print(f"  icon.ico: {os.path.getsize(ico_path):,} bytes ({len(ico_sizes)} sizes)")

print()
print(f"=== Done! All icons written to {out_dir} ===")
PYEOF

echo ""
ls -la "$OUTDIR"
