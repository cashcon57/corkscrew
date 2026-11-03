#!/usr/bin/env bash
# Builds the Windows launch trampoline Corkscrew embeds (see
# src-tauri/resources/trampoline/trampoline.c). Needs mingw-w64
# (macOS: brew install mingw-w64; Debian/Ubuntu: apt install gcc-mingw-w64-x86-64).
set -euo pipefail
cd "$(dirname "$0")/../src-tauri/resources/trampoline"
x86_64-w64-mingw32-gcc -municode -mwindows -O2 -s \
  -nostartfiles -Wl,-e,wWinMainCRTStartup_ \
  -Wl,--no-insert-timestamp -Wl,--build-id=none \
  -o corkscrew-trampoline.exe trampoline.c entry.c -lkernel32 -luser32
shasum -a 256 corkscrew-trampoline.exe 2>/dev/null || sha256sum corkscrew-trampoline.exe
