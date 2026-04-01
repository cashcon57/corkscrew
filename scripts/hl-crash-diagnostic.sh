#!/usr/bin/env bash
# Hogwarts Legacy Crash Diagnostic
# Systematically disables components to isolate what causes the crash.
#
# Usage: ./scripts/hl-crash-diagnostic.sh [step]
#   No args  = show menu
#   1        = disable UE4SS
#   2        = disable merged database
#   3        = disable all PAK mods
#   restore  = restore everything

set -euo pipefail

HL_DIR="$HOME/Library/Application Support/CrossOver/Bottles/Steam/drive_c/Program Files (x86)/Steam/steamapps/common/Hogwarts Legacy"
WIN64="$HL_DIR/Phoenix/Binaries/Win64"
MODS="$HL_DIR/Phoenix/Content/Paks/~mods"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

check_exists() {
    if [[ ! -d "$HL_DIR" ]]; then
        echo -e "${RED}Hogwarts Legacy not found at:${NC}"
        echo "  $HL_DIR"
        exit 1
    fi
}

show_status() {
    echo ""
    echo -e "${YELLOW}=== Current State ===${NC}"

    if [[ -f "$WIN64/dwmapi.dll" ]]; then
        echo -e "  UE4SS:           ${GREEN}ENABLED${NC} (dwmapi.dll present)"
    elif [[ -f "$WIN64/dwmapi.dll.diagnostic_bak" ]]; then
        echo -e "  UE4SS:           ${RED}DISABLED${NC} (dwmapi.dll renamed)"
    else
        echo -e "  UE4SS:           ${YELLOW}NOT INSTALLED${NC}"
    fi

    if [[ -f "$MODS/zMergedMods_P.pak" ]]; then
        echo -e "  Merged Database: ${GREEN}ENABLED${NC} (zMergedMods_P.pak present)"
    elif [[ -f "$MODS/zMergedMods_P.pak.diagnostic_bak" ]]; then
        echo -e "  Merged Database: ${RED}DISABLED${NC} (renamed)"
    else
        echo -e "  Merged Database: ${YELLOW}NOT PRESENT${NC}"
    fi

    local pak_count
    pak_count=$(find "$MODS" -maxdepth 1 -name "*.pak" 2>/dev/null | wc -l | tr -d ' ')
    local disabled_count
    disabled_count=$(find "$MODS" -maxdepth 1 -name "*.pak.diagnostic_bak" 2>/dev/null | wc -l | tr -d ' ')

    echo -e "  PAK Mods:        ${GREEN}${pak_count} active${NC}, ${disabled_count} disabled"
    echo ""
}

step1_disable_ue4ss() {
    echo -e "${YELLOW}Step 1: Disabling UE4SS...${NC}"
    if [[ -f "$WIN64/dwmapi.dll" ]]; then
        mv "$WIN64/dwmapi.dll" "$WIN64/dwmapi.dll.diagnostic_bak"
        echo -e "${GREEN}Done.${NC} UE4SS disabled (dwmapi.dll renamed)."
        echo ""
        echo "Now launch HL and try New Game."
        echo "  - If it WORKS: UE4SS is the crash cause. Try UE4SS v2.x or Wine-specific settings."
        echo "  - If it CRASHES: Run step 2 to test the merged database."
    else
        echo "dwmapi.dll not found — UE4SS may not be installed."
    fi
}

step1_restore_ue4ss() {
    if [[ -f "$WIN64/dwmapi.dll.diagnostic_bak" ]]; then
        mv "$WIN64/dwmapi.dll.diagnostic_bak" "$WIN64/dwmapi.dll"
        echo -e "${GREEN}UE4SS restored.${NC}"
    fi
}

step2_disable_merged_db() {
    echo -e "${YELLOW}Step 2: Disabling merged database...${NC}"
    # Restore UE4SS first
    step1_restore_ue4ss

    if [[ -f "$MODS/zMergedMods_P.pak" ]]; then
        mv "$MODS/zMergedMods_P.pak" "$MODS/zMergedMods_P.pak.diagnostic_bak"
        echo -e "${GREEN}Done.${NC} Merged database disabled."
        echo ""
        echo "Now launch HL and try New Game."
        echo "  - If it WORKS: The merged database has bad data. Re-install the collection."
        echo "  - If it CRASHES: Run step 3 to test without ALL PAK mods."
    else
        echo "zMergedMods_P.pak not found."
    fi
}

step2_restore_merged_db() {
    if [[ -f "$MODS/zMergedMods_P.pak.diagnostic_bak" ]]; then
        mv "$MODS/zMergedMods_P.pak.diagnostic_bak" "$MODS/zMergedMods_P.pak"
        echo -e "${GREEN}Merged database restored.${NC}"
    fi
}

step3_disable_all_paks() {
    echo -e "${YELLOW}Step 3: Disabling ALL PAK mods...${NC}"
    # Restore everything first
    step1_restore_ue4ss
    step2_restore_merged_db

    local count=0
    for pak in "$MODS/"*.pak; do
        [[ -f "$pak" ]] || continue
        mv "$pak" "${pak}.diagnostic_bak"
        count=$((count + 1))
    done
    echo -e "${GREEN}Done.${NC} Disabled $count PAK files."
    echo ""
    echo "Now launch HL and try New Game."
    echo "  - If it WORKS: One or more PAK mods cause the crash. Binary search needed."
    echo "  - If it CRASHES: The issue is outside Corkscrew's mods (UE4SS, Wine, or game itself)."
}

restore_all() {
    echo -e "${YELLOW}Restoring everything...${NC}"

    # Restore UE4SS
    step1_restore_ue4ss

    # Restore merged database
    step2_restore_merged_db

    # Restore all PAK files
    local count=0
    for bak in "$MODS/"*.pak.diagnostic_bak; do
        [[ -f "$bak" ]] || continue
        mv "$bak" "${bak%.diagnostic_bak}"
        count=$((count + 1))
    done
    if [[ $count -gt 0 ]]; then
        echo -e "${GREEN}Restored $count PAK files.${NC}"
    fi

    echo -e "${GREEN}All components restored.${NC}"
}

# --- Main ---
check_exists

STEP="${1:-menu}"

case "$STEP" in
    1)
        step1_disable_ue4ss
        show_status
        ;;
    2)
        step2_disable_merged_db
        show_status
        ;;
    3)
        step3_disable_all_paks
        show_status
        ;;
    restore|r)
        restore_all
        show_status
        ;;
    menu|*)
        echo ""
        echo -e "${YELLOW}Hogwarts Legacy Crash Diagnostic${NC}"
        echo "================================="
        echo ""
        echo "This script systematically disables components to find the crash cause."
        echo "Run each step, launch the game, and try New Game after each one."
        echo ""
        echo "  ./scripts/hl-crash-diagnostic.sh 1        Disable UE4SS only"
        echo "  ./scripts/hl-crash-diagnostic.sh 2        Disable merged database only"
        echo "  ./scripts/hl-crash-diagnostic.sh 3        Disable ALL PAK mods"
        echo "  ./scripts/hl-crash-diagnostic.sh restore   Restore everything"
        echo ""
        show_status
        ;;
esac
