# BG3 Native macOS Modding Internals — Research Spec

**Date:** 2026-04-28
**Purpose:** Reference for Tasks 4.2–4.5 (BG3 native macOS support in Corkscrew)
**Sources:** BGMM source (`LaughingLeader/BG3ModManager`), lslib (`Norbyte/lslib`), Nexus Mods app dev docs, bg3.wiki, community forums

---

## Section 1: BG3 Native macOS App Bundle

### Bundle Identifier

**TODO: verify** — The exact `CFBundleIdentifier` string is not publicly documented. Based on Larian's naming conventions it is likely `com.larian.baldursgate3`, but this must be confirmed by inspecting the installed `.app` bundle on a machine with BG3 installed:

```bash
/usr/libexec/PlistBuddy -c "Print :CFBundleIdentifier" \
  "/Applications/Baldur's Gate 3.app/Contents/Info.plist"
```

### Executable

The macOS app is named `"Baldur's Gate 3.app"`. Inside the bundle:

- Steam build executable: `"Baldur's Gate 3.app/Contents/MacOS/Baldur's Gate 3"`
- GOG build executable: `"Baldur's Gate 3.app/Contents/MacOS/Baldurs Gate 3 GOG"` (**TODO: verify exact filename — community reports suggest the GOG build ships a separate launcher executable alongside the Steam one**)

### Steam App ID

Steam App ID `1086940` maps to BG3 on all platforms including macOS. The native macOS build shipped September 21, 2024, via the same Steam appid.

### Distribution

- Steam: available, native Metal build (requires Metal 2.2)
- GOG: available, ships same native build with GOG launcher wrapper
- Mac App Store: not available — BG3 is not on the MAS

---

## Section 2: Native macOS Filesystem Layout

### Critical Finding: Documents, not Library/Application Support

Unlike many macOS-native games that use `~/Library/Application Support/`, BG3 on macOS stores user data in `~/Documents/`. This mirrors the Windows layout (which uses `%LOCALAPPDATA%`) but maps to the macOS equivalent for user-facing documents.

Multiple community sources confirm the paths below. **TODO: verify** by inspecting a live install — the game may use `~/Library/Application Support/` on some configurations.

### Mods directory

```
~/Documents/Larian Studios/Baldur's Gate 3/Mods/
```

Drop `.pak` files here directly (flat directory — no subdirectories). Subdirectories cause the game to reset `modsettings.lsx` on launch.

### Load order file

```
~/Documents/Larian Studios/Baldur's Gate 3/PlayerProfiles/Public/modsettings.lsx
```

The `Public` profile is the default and the one almost all users operate on.

### Profile data (read-only reference — we do not mutate these)

```
~/Documents/Larian Studios/Baldur's Gate 3/PlayerProfiles/Public/profile.lsf
~/Documents/Larian Studios/Baldur's Gate 3/PlayerProfiles/<ProfileName>/profile.lsf
```

### Save files (ignore during deploy — never touch)

```
~/Documents/Larian Studios/Baldur's Gate 3/PlayerProfiles/Public/Savegames/Story/
```

### Comparison: Windows layout

On Windows, everything lives under `%LOCALAPPDATA%\Larian Studios\Baldur's Gate 3\` (not `Documents`). The macOS layout uses `~/Documents/Larian Studios/Baldur's Gate 3/` instead. There is no `%LOCALAPPDATA%` equivalent involved.

---

## Section 3: modsettings.lsx Schema

The load-order file is standard Larian XML (LSX format). The game regenerates it on launch via the in-game mod manager if it detects corruption or if a mod fails to load — a corrupted or externally written file will be silently replaced. Write valid XML and keep the master entries intact to avoid this.

### Annotated Example

```xml
<?xml version="1.0" encoding="UTF-8"?>
<save>
  <!-- File format version — always 4.x for BG3 -->
  <version major="4" minor="0" revision="9" build="319"/>

  <region id="ModuleSettings">
    <!-- Only region we care about. The game may include other regions; preserve them. -->
    <node id="root">
      <children>

        <node id="Mods">
          <!-- ORDER MATTERS. "Last loaded wins" for conflicts. -->
          <!-- The game reads this list top-to-bottom at startup. -->
          <children>

            <!-- MASTER ENTRY #1 — MUST be first, NEVER remove or reorder -->
            <node id="ModuleShortDesc">
              <attribute id="Folder"        type="LSString" value="GustavDev"/>
              <attribute id="MD5"           type="LSString" value=""/>
              <attribute id="Name"          type="LSString" value="GustavDev"/>
              <attribute id="PublishHandle" type="uint64"   value="0"/>
              <attribute id="UUID"          type="guid"     value="28ac9ce2-2aba-8cda-b3b5-6e922f71b6b8"/>
              <attribute id="Version64"     type="int64"    value="36028797018963968"/>
            </node>

            <!-- MASTER ENTRY #2 — base game campaign module -->
            <node id="ModuleShortDesc">
              <attribute id="Folder"        type="LSString" value="Gustav"/>
              <attribute id="MD5"           type="LSString" value=""/>
              <attribute id="Name"          type="LSString" value="Gustav"/>
              <attribute id="PublishHandle" type="uint64"   value="0"/>
              <attribute id="UUID"          type="guid"     value="991c9c7a-fb80-40cb-8f0d-b92d4e80e9b1"/>
              <attribute id="Version64"     type="int64"    value="36029301681017806"/>
            </node>

            <!-- MASTER ENTRY #3 — shared developer content -->
            <!-- TODO: verify whether SharedDev is present in all fresh installs -->
            <node id="ModuleShortDesc">
              <attribute id="Folder"        type="LSString" value="SharedDev"/>
              <attribute id="MD5"           type="LSString" value=""/>
              <attribute id="Name"          type="LSString" value="SharedDev"/>
              <attribute id="PublishHandle" type="uint64"   value="0"/>
              <attribute id="UUID"          type="guid"     value="3d0c5ff8-c95d-c907-ff3e-34b204f1c630"/>
              <attribute id="Version64"     type="int64"    value="36028797022722506"/>
            </node>

            <!-- Community mods go AFTER the masters, in desired load order -->
            <node id="ModuleShortDesc">
              <attribute id="Folder"        type="LSString" value="MyModFolderName"/>
              <attribute id="MD5"           type="LSString" value=""/>
              <attribute id="Name"          type="LSString" value="My Mod Display Name"/>
              <attribute id="PublishHandle" type="uint64"   value="0"/>
              <attribute id="UUID"          type="guid"     value="aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"/>
              <attribute id="Version64"     type="int64"    value="36028797018963968"/>
            </node>

          </children>
        </node>

      </children>
    </node>
  </region>
</save>
```

### Master Entries (NEVER touch)

These entries represent Larian's own game data modules. Removing or reordering them crashes the game at the Larian logo.

| Folder | UUID | Version64 |
|--------|------|-----------|
| `GustavDev` | `28ac9ce2-2aba-8cda-b3b5-6e922f71b6b8` | `36028797018963968` |
| `Gustav` | `991c9c7a-fb80-40cb-8f0d-b92d4e80e9b1` | `36029301681017806` |
| `SharedDev` | `3d0c5ff8-c95d-c907-ff3e-34b204f1c630` | `36028797022722506` |

**TODO: verify** whether `Shared` (without `Dev`) is also a required entry in the release build. BGMM source references a `GustavX` UUID (`cb555efe-2d9e-131f-8195-a89329d218ea`) — this maps to the main campaign and may appear as an additional entry in the Definitive Edition or post-Patch 7 installations.

Implementation rule: on deploy, read the existing `modsettings.lsx`, identify all existing entries whose UUID matches the known master UUIDs, preserve them in their exact positions, and append new community mod entries after them.

### Version64 Encoding

Larian packs a four-component version into a signed 64-bit integer using bit shifts:

```
encode: Version64 = (Major << 55) | (Minor << 47) | (Revision << 31) | Build

decode:
  Major    = Version64 >> 55
  Minor    = (Version64 >> 47) & 0xFF
  Revision = (Version64 >> 31) & 0xFFFF
  Build    = Version64 & 0x7FFFFFFF
```

`36028797018963968` = version `1.0.0.0` (`1 << 55`). For a community mod with no meaningful version, using `36028797018963968` (1.0.0.0) is the correct default. Copy the value directly from `meta.lsx` — do not re-encode it.

### MD5 Field

The `MD5` attribute in `modsettings.lsx` is intentionally empty (`value=""`) for community mods. Larian's first-party modules carry a populated MD5 but community mods always use an empty string. The BGMM source confirms: "MD5 doesn't seem to actually be used" in mod settings generation.

### Line Endings

**TODO: verify** — LSX files are UTF-8 XML. On macOS the game almost certainly writes Unix line endings (`\n`). Write `\n` when generating this file.

---

## Section 4: .pak File Format (LSPK)

### Header

Magic signature: `LSPK` (bytes `4C 53 50 4B`, little-endian uint32 `0x4B50534C`).

| Version | Used by | Notes |
|---------|---------|-------|
| V7 | DOS1 era | 28-byte header, 32-bit offsets |
| V10 | DOS2 | 25-byte header, adds Flags + Priority |
| V13 | DOS2 DE | 35-byte header, adds MD5 |
| V15 | BG3 early access | 38-byte header, 64-bit offsets |
| V16 | BG3 | 40-byte header, adds NumParts |
| V18 | BG3 current | Optimized file entry layout |

BG3 release builds use V16 and V18. Nexus Mods App documents support for V15+. We need to handle V16 and V18 at minimum for meta reading.

### V18 Header Layout (current BG3 standard)

```
Offset  Size  Field
0       4     Magic ("LSPK")
4       4     Version (= 18)
8       8     FileListOffset (uint64)
16      4     FileListSize (uint32)
20      2     NumParts (uint16)
22      1     Flags (PackageFlags)
23      1     Priority (byte)
24      16    MD5[16]
```

### V18 File Entry (277 bytes, packed)

```
Offset  Size  Field
0       256   Name (null-terminated UTF-8)
256     4     OffsetInFile1 (uint32, lower 32 bits)
260     2     OffsetInFile2 (uint16, upper 16 bits → combined: uint48)
262     1     ArchivePart (byte)
263     1     Flags (byte, lower nibble = compression method)
264     4     SizeOnDisk (uint32)
268     4     UncompressedSize (uint32)
```

### Compression

The `Flags` byte in the file entry encodes the compression method in the lower 4 bits:

| Value | Method |
|-------|--------|
| 0 | None |
| 1 | Zlib |
| 2 | LZ4 |
| 3 | Zstd |

BG3 paks use LZ4 and Zstd in practice. The `Solid` package flag (`0x04`) indicates a single shared LZ4 compressed stream for all files (used for some Larian data paks). Community mods are typically per-file compressed or uncompressed.

A deleted file is marked by `(OffsetInFile & 0x0000ffffffffffff) == 0xbeefdeadbeef`.

### Where meta.lsx Lives in the Pak

Community mod paks follow the path convention:

```
Mods/<FolderName>/meta.lsx
```

The pattern used by BGMM to locate it: `^Mods/([^/]+)/meta.lsx` (regex). When multiple matches exist, BGMM prefers the one whose directory name matches the pak filename (without extension).

### Deploy Strategy

For Corkscrew's deploy pipeline: **do NOT extract the pak**. Drop the `.pak` file as-is into the Mods directory. We only need to read `meta.lsx` from inside the pak to populate the `modsettings.lsx` entry. This is a "meta reader only" use case — write a minimal LSPK parser that:

1. Reads the header to find FileListOffset
2. Reads the file table to locate the entry whose name matches `^Mods/.+/meta.lsx`
3. Decompresses that entry (LZ4 or Zstd, depending on Flags)
4. Parses the resulting XML

We do NOT need a full pak extractor for Phase 4.

---

## Section 5: meta.lsx Schema

Located at `Mods/<FolderName>/meta.lsx` inside the `.pak`. It is UTF-8 XML in Larian LSX format.

### Annotated Example

```xml
<?xml version="1.0" encoding="UTF-8"?>
<save>
  <version major="4" minor="0" revision="0" build="0"/>
  <region id="Config">
    <node id="root">
      <children>

        <node id="Dependencies">
          <!-- Optional: lists other mods this mod requires -->
          <!-- Each dependency is a ModuleShortDesc child node -->
          <!-- We parse this in Phase 4 for display only; enforcement deferred to later phase -->
        </node>

        <node id="ModuleInfo">
          <attribute id="Author"                    type="LSString"    value="AuthorName"/>
          <attribute id="CharacterCreationLevelName" type="LSString"   value=""/>
          <attribute id="Description"               type="LSString"    value="A description of the mod."/>
          <!-- Folder MUST exactly match the directory name inside the pak -->
          <attribute id="Folder"                    type="LSString"    value="MyModFolderName"/>
          <attribute id="LobbyLevelName"            type="LSString"    value=""/>
          <!-- MD5 is populated by Larian's official tools; community mods leave it empty -->
          <attribute id="MD5"                       type="LSString"    value=""/>
          <attribute id="MainMenuBackgroundVideo"   type="LSString"    value=""/>
          <attribute id="MenuLevelName"             type="LSString"    value=""/>
          <!-- Name is the human-readable display name -->
          <attribute id="Name"                      type="LSString"    value="My Mod Display Name"/>
          <attribute id="NumPlayers"                type="uint8"       value="4"/>
          <attribute id="PhotoBooth"                type="LSString"    value=""/>
          <attribute id="StartupLevelName"          type="LSString"    value=""/>
          <attribute id="Tags"                      type="LSString"    value=""/>
          <!-- Type should be "Add-on" for community mods -->
          <attribute id="Type"                      type="FixedString" value="Add-on"/>
          <!-- UUID is a GUID — must be unique per mod -->
          <attribute id="UUID"                      type="FixedString" value="aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"/>
          <!-- Version64 in Larian's packed int64 format (see Section 3) -->
          <attribute id="Version64"                 type="int64"       value="36028797018963968"/>
        </node>

      </children>
    </node>
  </region>
</save>
```

### Fields to Extract for modsettings.lsx Entry

| meta.lsx field | modsettings.lsx attribute | Notes |
|----------------|--------------------------|-------|
| `Folder` | `Folder` | Direct copy |
| `Name` | `Name` | XML-escape special characters |
| `UUID` | `UUID` | Direct copy |
| `Version64` | `Version64` | Direct copy as int64 string |
| _(not in meta)_ | `MD5` | Always write `""` |
| _(not in meta)_ | `PublishHandle` | Always write `"0"` |

The `MD5` field in `modsettings.lsx` is always empty for community mods. Only Larian's official content carries a populated MD5. This is confirmed by BGMM source: the field is stored but described as apparently unused.

---

## Section 6: BG3 Script Extender (BG3SE)

### Official Support Status

BG3SE (Norbyte's `bg3se`) has **no official macOS support**. The official releases target Windows only.

### Community macOS Port

A third-party port exists: `tdimino/bg3se-macos` on GitHub. This is a native macOS rebuild that compiles to a universal binary (`arm64` + `x86_64`). Build artifact: `build/lib/libbg3se.dylib`.

Installation uses a launch wrapper script: the user sets Steam launch options to invoke `bg3w.sh %command%`, which injects the dylib before BG3 starts. There is no official installer.

### Detection

For detect-only (Task 4.6 is detection, not installation):

Look for the presence of `libbg3se.dylib` in a known location relative to the app bundle. **TODO: verify the exact installed path** — community reports suggest it sits alongside the game executable:

```
Baldur's Gate 3.app/Contents/MacOS/libbg3se.dylib
```

or in the game's root data directory. Check both. Absence of the file means BG3SE is not installed — surface a warning if the mod being installed declares SE dependency in its `meta.lsx` description or name.

### Lag vs. Windows

The community macOS port significantly lags the Windows release. Many SE-dependent mods will silently fail or require an older SE version. Surface a visible caution in the UI when deploying any mod with "Script Extender" in its name or description.

---

## Section 7: Deploy Procedure

### Install Algorithm

```
1. Validate pak
   - Open the .pak file, read LSPK header, verify magic bytes
   - Confirm version is 15, 16, or 18 (reject older formats)

2. Read meta.lsx from pak
   - Scan file table for entry matching ^Mods/[^/]+/meta.lsx
   - Decompress the entry (LZ4 or Zstd per entry Flags byte)
   - Parse XML, extract from ModuleInfo node:
       Folder, Name, UUID, Version64

3. Validate extracted data
   - UUID must be a valid GUID and must NOT match any master UUID
     (GustavDev, Gustav, SharedDev — reject silently if it does)
   - Folder must be a valid directory name (no path separators, no ..)

4. Copy .pak to Mods directory
   - Destination: ~/Documents/Larian Studios/Baldur's Gate 3/Mods/<original_filename>.pak
   - Use atomic rename (write to temp, then rename) to prevent partial writes
   - Refuse if game process is running (check for BG3 process by name)

5. Mutate modsettings.lsx
   a. Snapshot: copy existing modsettings.lsx to a backup before any write
      (use auto_snapshot_before_destructive() pattern from Corkscrew's rollback.rs)
   b. Parse existing modsettings.lsx with quick-xml
   c. Locate <node id="Mods"> children list
   d. Check for existing entry with matching UUID — if found, update in place
   e. If not found, append new <node id="ModuleShortDesc"> AFTER all master entries
      with attributes: Folder, MD5="", Name, PublishHandle="0", UUID, Version64
   f. Write back with Unix line endings, UTF-8, XML declaration preserved

6. Record in Corkscrew database
   - Store pak path, UUID, Folder, Name, Version64 for later uninstall/redeploy
```

### Uninstall Algorithm

```
1. Snapshot modsettings.lsx before mutation

2. Parse modsettings.lsx
   - Find <node id="ModuleShortDesc"> where UUID attribute matches the mod's UUID
   - Safety check: if UUID is a master UUID — abort, do not remove

3. Remove the matching node from the XML

4. Write back modsettings.lsx

5. Delete .pak from Mods directory
   - If file is locked (game running), fail with clear error

6. Remove from Corkscrew database
```

---

## Section 8: Edge Cases and Gotchas

### Game Running → File Locking

BG3 keeps pak files open while running. Deploying or removing paks while the game is open will either fail silently (the old file stays open) or cause a partial read. Check for a running BG3 process before any file operation — this matches Corkscrew's existing `game_lock` pattern.

### modsettings.lsx Overwrite on Launch

The in-game mod manager (added in Patch 7) regenerates `modsettings.lsx` on game launch if:
- A mod in the list fails to load (pak not found or corrupted)
- The file has a subdirectory-based mod reference (subdirs in `Mods/` trigger a reset)
- The user interacts with the in-game mod manager (which writes its own ordering)

Consequence for Corkscrew: after deploying, if the user opens the in-game manager and clicks anything, our written load order may be overwritten. This is a known limitation of all external BG3 mod managers. Mitigation: encourage users to not use the in-game manager if using Corkscrew (same guidance BGMM gives).

### Profile Selection (Public vs. Custom)

Most users use the `Public` profile. Corkscrew should default to `Public`. If a non-Public profile directory exists, surface a selector in the UI. The `modsettings.lsx` path changes to:

```
~/Documents/Larian Studios/Baldur's Gate 3/PlayerProfiles/<ProfileName>/modsettings.lsx
```

For Phase 4, hard-code `Public`. Add profile selection in a later task.

### Pak Naming Conflicts

If two mods have the same pak filename but different UUIDs, dropping both into `Mods/` will cause one to overwrite the other. Detect filename conflicts on deploy and either rename the incoming file (e.g., append UUID prefix) or warn the user.

### BG3SE-Dependent Mods

`meta.lsx` has no explicit "requires script extender" flag. Detection heuristics:
- Mod name or description contains "Script Extender" or "SE required"
- Mod pak contains files with `.dll` extension (SE-loaded native libraries)

If BG3SE is not detected (see Section 6) and a mod appears SE-dependent, show a warning before deploy.

### Dependencies in meta.lsx

The `<node id="Dependencies">` block lists other mods as `ModuleShortDesc` children. We read them for display in Phase 4 but do not enforce them. Enforcement is deferred (same pattern as Stardew in Task 3.8).

### Pak Format Version

Reject paks with LSPK version older than 15 (DOS1/DOS2 era mods, not BG3 compatible). Show a clear error rather than silently failing to parse `meta.lsx`.

---

## Section 9: Implementation Notes for Corkscrew

### XML Parsing — quick-xml

`quick-xml` is already in Corkscrew's dependency tree. Use it for both reading and writing `modsettings.lsx` and `meta.lsx`. Both files are well-formed XML with no special DTD or namespace requirements. Use `quick-xml`'s event-based reader for the pak meta extraction (parse in-memory from the decompressed bytes without touching the filesystem).

### LSPK Reader

No suitable Rust crate exists on crates.io for LSPK (the `lspk` name is taken by an unrelated project; no maintained BG3-compatible crate was found as of this writing). Write a minimal `bg3_pak.rs` module with:

- `read_header(path) -> PakHeader` — reads magic, version, file table offset
- `find_file(path, pattern: &Regex) -> Option<Vec<u8>>` — scans file table, extracts and decompresses one file
- Compression backends: LZ4 via `lz4_flex` crate (already in use by Corkscrew for WJ pipeline), Zstd via `zstd` crate (evaluate; may already be present)

The reader only needs to handle V15/V16/V18. Implement V18 first (current BG3 standard), then add V15/V16 fallback.

### Dependency Candidates (evaluate in Tasks 4.2/4.3, do NOT add during this spike)

| Crate | Purpose | Status |
|-------|---------|--------|
| `lz4_flex` | LZ4 decompression for pak entries | Likely already in Corkscrew (WJ pipeline) |
| `zstd` | Zstd decompression for pak entries | Evaluate |
| `quick-xml` | LSX/modsettings.lsx read+write | Already present |
| `uuid` | UUID validation | Already present |

### Rust Module Plan

- `src-tauri/src/bg3_pak.rs` — LSPK reader (header parse + single-file extraction)
- `src-tauri/src/plugins/baldurs_gate3.rs` — BG3 `GamePlugin` impl, uses `bg3_pak` for meta reading, handles modsettings.lsx deploy logic

### Test Coverage

Write unit tests with a minimal hand-crafted LSPK V18 pak containing a `meta.lsx` entry. Verify:
- Header parse
- File table scan finds `Mods/TestMod/meta.lsx`
- Decompression produces valid XML bytes
- `meta.lsx` parse extracts correct Folder/Name/UUID/Version64
- modsettings.lsx round-trip: parse → append entry → serialize → re-parse → entry present, masters untouched
