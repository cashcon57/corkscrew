#!/usr/bin/env bash
# Hook: SessionStart (compact) — Re-anchor context after compaction
# Outputs context reminders that Claude receives after context compression.

cat <<'REMINDER'
Context was just compacted. Re-anchor by:
1. Check the conversation summary for in-progress work state
2. Re-read relevant memory files if working on:
   - Corkscrew backend: check MEMORY.md for key patterns and module list
   - SSEEngineFixesForWine: re-read sseef-wine.md for crash history and architecture
   - Cross-repo integration: re-read engine-fixes-wine.md
3. Verify any version numbers or file paths referenced in the compressed context
REMINDER
