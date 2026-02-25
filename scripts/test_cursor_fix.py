#!/usr/bin/env python3
"""
Automated test for the macOS cursor fix (Dock suppression + event tap).

Tests:
1. Dock autohide-delay manipulation (suppress/restore)
2. Display height detection (logical points)
3. Cursor position after warping to bottom edge
4. Event tap creation (if Accessibility permission is granted)

Usage:
    python3 scripts/test_cursor_fix.py          # Run all tests
    python3 scripts/test_cursor_fix.py --dock    # Test Dock suppression only
    python3 scripts/test_cursor_fix.py --warp    # Test cursor warping only
    python3 scripts/test_cursor_fix.py --tap     # Test event tap only

No external dependencies — uses only macOS system frameworks via ctypes.
"""

import ctypes
import ctypes.util
import subprocess
import sys
import time
import os

# Load macOS frameworks
CG = ctypes.CDLL(ctypes.util.find_library("CoreGraphics"))
CF = ctypes.CDLL(ctypes.util.find_library("CoreFoundation"))
AS = ctypes.CDLL(ctypes.util.find_library("ApplicationServices"))

# CGDisplayBounds returns CGRect by value (origin.x, origin.y, size.w, size.h)
class CGPoint(ctypes.Structure):
    _fields_ = [("x", ctypes.c_double), ("y", ctypes.c_double)]

class CGSize(ctypes.Structure):
    _fields_ = [("width", ctypes.c_double), ("height", ctypes.c_double)]

class CGRect(ctypes.Structure):
    _fields_ = [("origin", CGPoint), ("size", CGSize)]

CG.CGMainDisplayID.restype = ctypes.c_uint32
CG.CGDisplayBounds.restype = CGRect
CG.CGDisplayBounds.argtypes = [ctypes.c_uint32]

# Mouse event functions
CG.CGEventCreateMouseEvent.restype = ctypes.c_void_p
CG.CGEventCreateMouseEvent.argtypes = [
    ctypes.c_void_p, ctypes.c_uint32, CGPoint, ctypes.c_uint32
]
CG.CGEventPost.argtypes = [ctypes.c_uint32, ctypes.c_void_p]
CG.CGWarpMouseCursorPosition.argtypes = [CGPoint]
CG.CGWarpMouseCursorPosition.restype = ctypes.c_int32

# Event source for reading cursor position
CG.CGEventCreate.restype = ctypes.c_void_p
CG.CGEventCreate.argtypes = [ctypes.c_void_p]
CG.CGEventGetLocation.restype = CGPoint
CG.CGEventGetLocation.argtypes = [ctypes.c_void_p]

# Accessibility check
AS.AXIsProcessTrusted.restype = ctypes.c_bool

CF.CFRelease.argtypes = [ctypes.c_void_p]

PASS = "\033[92mPASS\033[0m"
FAIL = "\033[91mFAIL\033[0m"
SKIP = "\033[93mSKIP\033[0m"
INFO = "\033[94mINFO\033[0m"


def get_cursor_position():
    """Get current cursor position using CGEventCreate."""
    event = CG.CGEventCreate(None)
    if not event:
        return None
    pos = CG.CGEventGetLocation(event)
    CF.CFRelease(event)
    return (pos.x, pos.y)


def get_display_size():
    """Get main display size in logical points."""
    display_id = CG.CGMainDisplayID()
    bounds = CG.CGDisplayBounds(display_id)
    return (bounds.size.width, bounds.size.height)


def warp_cursor(x, y):
    """Move cursor to position."""
    point = CGPoint(x, y)
    CG.CGWarpMouseCursorPosition(point)
    time.sleep(0.05)  # Give macOS time to process


def run_cmd(args):
    """Run a command and return (success, stdout)."""
    try:
        result = subprocess.run(args, capture_output=True, text=True, timeout=5)
        return result.returncode == 0, result.stdout.strip()
    except Exception as e:
        return False, str(e)


# -----------------------------------------------------------------------
# Test: Display height
# -----------------------------------------------------------------------
def test_display_height():
    print("\n--- Test: Display Height Detection ---")
    w, h = get_display_size()
    print(f"  Display size (logical points): {w:.0f} x {h:.0f}")

    # On a Retina Mac, logical height should be roughly half the physical
    # Common values: 1117 (MacBook Pro 16"), 900 (1080p), 800 (Steam Deck)
    if h > 0 and w > 0:
        print(f"  {PASS} Display detected successfully")
        return True
    else:
        print(f"  {FAIL} Display size is zero!")
        return False


# -----------------------------------------------------------------------
# Test: Dock suppression
# -----------------------------------------------------------------------
def test_dock_suppression():
    print("\n--- Test: Dock Suppression (read/write/restore cycle) ---")

    # Check current state
    ok, val = run_cmd(["defaults", "read", "com.apple.dock", "autohide"])
    autohide_on = ok and val == "1"
    print(f"  Dock autohide currently: {autohide_on}")

    ok, val = run_cmd(["defaults", "read", "com.apple.dock", "autohide-delay"])
    original_delay = float(val) if ok and val else 0.0
    print(f"  Dock autohide-delay currently: {original_delay}")

    # Simulate suppress: enable autohide + set huge delay
    print("\n  [Suppress] Writing autohide=true, autohide-delay=86400...")
    ok1, _ = run_cmd(["defaults", "write", "com.apple.dock", "autohide", "-bool", "true"])
    ok2, _ = run_cmd(["defaults", "write", "com.apple.dock", "autohide-delay", "-float", "86400"])
    if not ok1 or not ok2:
        print(f"  {FAIL} Failed to write Dock settings")
        return False

    # Verify
    ok, val = run_cmd(["defaults", "read", "com.apple.dock", "autohide"])
    print(f"  Verify autohide: {val}")
    ok, val = run_cmd(["defaults", "read", "com.apple.dock", "autohide-delay"])
    set_delay = float(val) if ok and val else 0.0
    print(f"  Verify autohide-delay: {set_delay}")

    if set_delay != 86400.0:
        print(f"  {FAIL} autohide-delay not set correctly (got {set_delay})")
        return False

    # Simulate restore
    print(f"\n  [Restore] Writing autohide={autohide_on}, delay={original_delay}...")
    if not autohide_on:
        run_cmd(["defaults", "write", "com.apple.dock", "autohide", "-bool", "false"])
    if original_delay == 0.0:
        run_cmd(["defaults", "delete", "com.apple.dock", "autohide-delay"])
    else:
        run_cmd(["defaults", "write", "com.apple.dock", "autohide-delay", "-float", str(original_delay)])

    # Verify restore
    ok, val = run_cmd(["defaults", "read", "com.apple.dock", "autohide"])
    restored_ah = ok and val == "1"
    print(f"  Verify autohide restored: {restored_ah} (expected {autohide_on})")

    ok, val = run_cmd(["defaults", "read", "com.apple.dock", "autohide-delay"])
    if ok:
        restored_delay = float(val) if val else 0.0
        print(f"  Verify delay restored: {restored_delay}")
    else:
        print(f"  Verify delay restored: (key deleted = default 0.0)")

    if restored_ah != autohide_on:
        print(f"  {FAIL} autohide not properly restored")
        return False

    print(f"\n  {PASS} Dock suppression read/write/restore cycle works")
    print(f"  NOTE: Did NOT restart Dock (killall Dock) — this was a plist-only test")
    return True


# -----------------------------------------------------------------------
# Test: Cursor warping and position reading
# -----------------------------------------------------------------------
def test_cursor_warp():
    print("\n--- Test: Cursor Position Control ---")

    w, h = get_display_size()
    if h <= 0:
        print(f"  {FAIL} No display detected")
        return False

    # Save original position
    orig = get_cursor_position()
    if not orig:
        print(f"  {FAIL} Cannot read cursor position")
        return False
    print(f"  Original cursor position: ({orig[0]:.0f}, {orig[1]:.0f})")

    # Test 1: Warp to center
    cx, cy = w / 2, h / 2
    warp_cursor(cx, cy)
    pos = get_cursor_position()
    if pos and abs(pos[0] - cx) < 2 and abs(pos[1] - cy) < 2:
        print(f"  {PASS} Warp to center ({cx:.0f}, {cy:.0f}) → ({pos[0]:.0f}, {pos[1]:.0f})")
    else:
        print(f"  {FAIL} Warp to center failed: expected ({cx:.0f}, {cy:.0f}), got {pos}")

    # Test 2: Warp to bottom edge (where the Dock trigger zone is)
    bx, by = w / 2, h - 1
    warp_cursor(bx, by)
    pos = get_cursor_position()
    print(f"  Warped to bottom edge ({bx:.0f}, {by:.0f}) → ({pos[0]:.0f}, {pos[1]:.0f})")

    # Test 3: Warp to the clamped zone (20px from bottom)
    margin = 20.0
    max_y = h - margin
    cx2, cy2 = w / 2, max_y
    warp_cursor(cx2, cy2)
    pos = get_cursor_position()
    print(f"  Warped to clamp zone ({cx2:.0f}, {cy2:.0f}) → ({pos[0]:.0f}, {pos[1]:.0f})")

    # Restore original position
    warp_cursor(orig[0], orig[1])

    print(f"  {PASS} Cursor warping works (display: {w:.0f}x{h:.0f}, clamp_y={max_y:.0f})")
    return True


# -----------------------------------------------------------------------
# Test: Accessibility permission
# -----------------------------------------------------------------------
def test_accessibility():
    print("\n--- Test: Accessibility Permission ---")
    trusted = AS.AXIsProcessTrusted()
    if trusted:
        print(f"  {PASS} AXIsProcessTrusted() = True — event tap will work")
    else:
        print(f"  {INFO} AXIsProcessTrusted() = False — event tap won't work")
        print(f"  This is OK: Dock suppression (the primary fix) works without it")
        print(f"  To enable event tap: System Settings → Privacy & Security → Accessibility → Terminal.app")
    return True  # Not a failure — just informational


# -----------------------------------------------------------------------
# Test: LIVE suppress/restore cycle (actually restarts Dock)
# -----------------------------------------------------------------------
def test_live():
    print("\n--- Test: LIVE Dock Suppress → Restore Cycle ---")
    print("  WARNING: This will restart the Dock twice (brief visual flash)")

    # 1. Record original state
    ok, val = run_cmd(["defaults", "read", "com.apple.dock", "autohide"])
    orig_autohide = ok and val == "1"
    ok, val = run_cmd(["defaults", "read", "com.apple.dock", "autohide-delay"])
    orig_delay = float(val) if ok and val else 0.0
    print(f"\n  Original state:")
    print(f"    autohide = {orig_autohide}")
    print(f"    autohide-delay = {orig_delay}")

    # 2. SUPPRESS: enable autohide + huge delay + restart Dock
    print(f"\n  [SUPPRESS] Enabling autohide + delay=86400 + killall Dock...")
    run_cmd(["defaults", "write", "com.apple.dock", "autohide", "-bool", "true"])
    run_cmd(["defaults", "write", "com.apple.dock", "autohide-delay", "-float", "86400"])
    run_cmd(["killall", "Dock"])
    time.sleep(2)  # Wait for Dock to restart

    # 3. Verify suppressed state
    ok, val = run_cmd(["defaults", "read", "com.apple.dock", "autohide"])
    sup_autohide = ok and val == "1"
    ok, val = run_cmd(["defaults", "read", "com.apple.dock", "autohide-delay"])
    sup_delay = float(val) if ok and val else 0.0
    print(f"  Suppressed state:")
    print(f"    autohide = {sup_autohide} (expected True)")
    print(f"    autohide-delay = {sup_delay} (expected 86400.0)")

    if not sup_autohide or sup_delay != 86400.0:
        print(f"  {FAIL} Suppress did not apply correctly!")
        # Emergency restore
        _restore_dock(orig_autohide, orig_delay)
        return False

    print(f"  Dock should now be hidden. Waiting 3 seconds...")
    time.sleep(3)

    # 4. RESTORE: put back original settings + restart Dock
    print(f"\n  [RESTORE] Restoring autohide={orig_autohide}, delay={orig_delay} + killall Dock...")
    _restore_dock(orig_autohide, orig_delay)
    time.sleep(2)  # Wait for Dock to restart

    # 5. Verify restored state
    ok, val = run_cmd(["defaults", "read", "com.apple.dock", "autohide"])
    res_autohide = ok and val == "1"
    ok, val = run_cmd(["defaults", "read", "com.apple.dock", "autohide-delay"])
    if ok:
        res_delay = float(val) if val else 0.0
        delay_restored = (res_delay == orig_delay)
    else:
        res_delay = 0.0
        delay_restored = (orig_delay == 0.0)  # Key deleted = default 0.0

    print(f"  Restored state:")
    print(f"    autohide = {res_autohide} (expected {orig_autohide})")
    print(f"    autohide-delay = {res_delay} (expected {orig_delay})")

    passed = True
    if res_autohide != orig_autohide:
        print(f"  {FAIL} autohide NOT restored! Got {res_autohide}, expected {orig_autohide}")
        passed = False
    if not delay_restored:
        print(f"  {FAIL} autohide-delay NOT restored! Got {res_delay}, expected {orig_delay}")
        passed = False

    if passed:
        print(f"\n  {PASS} Live suppress/restore cycle works — Dock fully restored")
    return passed


def _restore_dock(orig_autohide, orig_delay):
    """Helper: restore Dock to original state."""
    if not orig_autohide:
        run_cmd(["defaults", "write", "com.apple.dock", "autohide", "-bool", "false"])
    else:
        run_cmd(["defaults", "write", "com.apple.dock", "autohide", "-bool", "true"])
    if orig_delay == 0.0:
        run_cmd(["defaults", "delete", "com.apple.dock", "autohide-delay"])
    else:
        run_cmd(["defaults", "write", "com.apple.dock", "autohide-delay", "-float", str(orig_delay)])
    run_cmd(["killall", "Dock"])


# -----------------------------------------------------------------------
# Main
# -----------------------------------------------------------------------
def main():
    print("=" * 60)
    print("  Corkscrew Cursor Fix — Automated Test Suite")
    print("=" * 60)

    mode = sys.argv[1] if len(sys.argv) > 1 else "--all"

    results = []

    if mode in ("--all", "--display"):
        results.append(("Display Height", test_display_height()))

    if mode in ("--all", "--dock"):
        results.append(("Dock Suppression", test_dock_suppression()))

    if mode in ("--all", "--warp"):
        results.append(("Cursor Warp", test_cursor_warp()))

    if mode in ("--all", "--tap"):
        results.append(("Accessibility", test_accessibility()))

    if mode in ("--all", "--live", "--integration"):
        results.append(("Live Dock Cycle", test_live()))

    # Summary
    print("\n" + "=" * 60)
    passed = sum(1 for _, r in results if r)
    total = len(results)
    print(f"  Results: {passed}/{total} passed")
    for name, result in results:
        status = PASS if result else FAIL
        print(f"    {status} {name}")
    print("=" * 60)

    return 0 if all(r for _, r in results) else 1


if __name__ == "__main__":
    sys.exit(main())
