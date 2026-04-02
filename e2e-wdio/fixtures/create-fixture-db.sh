#!/bin/bash
# Creates a pre-seeded SQLite fixture database for E2E testing.
# Run this to regenerate: ./fixtures/create-fixture-db.sh

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DB="$SCRIPT_DIR/test-mods.db"

rm -f "$DB"

sqlite3 "$DB" <<'SQL'
-- Schema version (must match current migration level)
CREATE TABLE schema_version (version INTEGER NOT NULL);
INSERT INTO schema_version VALUES (22);

-- Core mods table
CREATE TABLE installed_mods (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    game_id         TEXT    NOT NULL,
    bottle_name     TEXT    NOT NULL,
    nexus_mod_id    INTEGER,
    name            TEXT    NOT NULL,
    version         TEXT    NOT NULL,
    archive_name    TEXT    NOT NULL,
    installed_files TEXT    NOT NULL,
    installed_at    TEXT    NOT NULL,
    enabled         INTEGER NOT NULL DEFAULT 1,
    nexus_file_id   INTEGER,
    source_url      TEXT,
    staging_path    TEXT,
    install_priority INTEGER NOT NULL DEFAULT 0,
    fomod_selections TEXT,
    collection_name TEXT,
    user_notes      TEXT,
    user_tags       TEXT,
    auto_category   TEXT,
    source_type     TEXT NOT NULL DEFAULT 'manual',
    collection_optional INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_installed_mods_game_bottle ON installed_mods (game_id, bottle_name);

-- Profiles
CREATE TABLE profiles (
    name        TEXT NOT NULL,
    game_id     TEXT NOT NULL,
    bottle_name TEXT NOT NULL,
    is_active   INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (name, game_id, bottle_name)
);

-- Deployment manifest
CREATE TABLE deployment_manifest (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    game_id     TEXT NOT NULL,
    bottle_name TEXT NOT NULL,
    mod_id      INTEGER NOT NULL,
    source_path TEXT NOT NULL,
    dest_path   TEXT NOT NULL,
    deployed_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Conflicts
CREATE TABLE file_conflicts (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    game_id         TEXT NOT NULL,
    bottle_name     TEXT NOT NULL,
    relative_path   TEXT NOT NULL,
    winning_mod_id  INTEGER NOT NULL,
    losing_mod_ids  TEXT NOT NULL DEFAULT '[]'
);

-- Config table
CREATE TABLE IF NOT EXISTS app_config (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Mod snapshots for rollback
CREATE TABLE mod_snapshots (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    game_id     TEXT NOT NULL,
    bottle_name TEXT NOT NULL,
    snapshot_at TEXT NOT NULL DEFAULT (datetime('now')),
    snapshot_data TEXT NOT NULL,
    reason      TEXT
);

-- Steam depot history
CREATE TABLE steam_depot_history (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    game_id      TEXT NOT NULL,
    app_id       TEXT NOT NULL,
    depot_id     TEXT NOT NULL,
    manifest_id  TEXT NOT NULL,
    build_id     TEXT NOT NULL DEFAULT '',
    game_version TEXT,
    captured_at  TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(game_id, depot_id, manifest_id)
);

-- Collections
CREATE TABLE installed_collections (
    slug              TEXT NOT NULL,
    name              TEXT NOT NULL,
    game_domain       TEXT NOT NULL,
    game_id           TEXT NOT NULL,
    bottle_name       TEXT NOT NULL,
    author            TEXT,
    latest_revision   INTEGER,
    installed_revision INTEGER,
    mod_count         INTEGER,
    download_size     INTEGER,
    status            TEXT NOT NULL DEFAULT 'installed',
    PRIMARY KEY (slug, game_id, bottle_name)
);

-- Download queue
CREATE TABLE download_queue (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    game_id     TEXT NOT NULL,
    bottle_name TEXT NOT NULL,
    mod_name    TEXT NOT NULL,
    file_name   TEXT,
    url         TEXT,
    status      TEXT NOT NULL DEFAULT 'pending',
    added_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Custom games
CREATE TABLE custom_games (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    bottle_name TEXT NOT NULL,
    game_path   TEXT NOT NULL,
    exe_path    TEXT,
    mod_dir     TEXT,
    nexus_slug  TEXT,
    steam_app_id TEXT
);

-- Notifications / error events
CREATE TABLE error_events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    event_type  TEXT NOT NULL,
    message     TEXT NOT NULL,
    details     TEXT,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Plugin rules
CREATE TABLE plugin_rules (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    game_id     TEXT NOT NULL,
    bottle_name TEXT NOT NULL,
    plugin_name TEXT NOT NULL,
    rule_type   TEXT NOT NULL,
    target      TEXT NOT NULL
);

-- Mod dependencies
CREATE TABLE mod_dependencies (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    mod_id          INTEGER NOT NULL,
    depends_on_id   INTEGER NOT NULL,
    dependency_type TEXT NOT NULL DEFAULT 'required'
);

-- FOMOD recipes
CREATE TABLE fomod_recipes (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    game_id     TEXT NOT NULL,
    archive_hash TEXT NOT NULL,
    selections  TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

--------------------------------------------------------------------
-- SEED DATA: Realistic test scenarios
--------------------------------------------------------------------

-- Active profile
INSERT INTO profiles VALUES ('default', 'skyrimse', 'Test Bottle', 1, '2026-01-15T10:00:00Z');
INSERT INTO profiles VALUES ('modded', 'skyrimse', 'Test Bottle', 0, '2026-02-01T12:00:00Z');

-- Skyrim SE mods: mix of enabled, disabled, various sources, conflicts, priorities
-- Essential framework mods (high priority, always enabled)
INSERT INTO installed_mods (game_id, bottle_name, nexus_mod_id, name, version, archive_name, installed_files, installed_at, enabled, staging_path, install_priority, auto_category, source_type, user_tags)
VALUES ('skyrimse', 'Test Bottle', 32444, 'Address Library for SKSE', '11', 'AddressLibrary.7z', '["SKSE/Plugins/versionlib-0-0-0-0.bin"]', '2026-01-15T10:01:00Z', 1, '/tmp/e2e-staging/1', 1, 'Utilities', 'nexus', '["essential"]');

INSERT INTO installed_mods (game_id, bottle_name, nexus_mod_id, name, version, archive_name, installed_files, installed_at, enabled, staging_path, install_priority, auto_category, source_type, user_tags)
VALUES ('skyrimse', 'Test Bottle', 12604, 'SkyUI', '5.2SE', 'SkyUI_5_2SE.7z', '["Data/SkyUI_SE.bsa","Data/SkyUI_SE.esp"]', '2026-01-15T10:02:00Z', 1, '/tmp/e2e-staging/2', 10, 'UI', 'nexus', '["essential"]');

INSERT INTO installed_mods (game_id, bottle_name, nexus_mod_id, name, version, archive_name, installed_files, installed_at, enabled, staging_path, install_priority, auto_category, source_type)
VALUES ('skyrimse', 'Test Bottle', 266, 'Unofficial Skyrim SE Patch', '4.3.2', 'USSEP.7z', '["Data/Unofficial Skyrim Special Edition Patch.bsa","Data/Unofficial Skyrim Special Edition Patch.esp"]', '2026-01-15T10:03:00Z', 1, '/tmp/e2e-staging/3', 5, 'Bug Fixes', 'nexus');

-- Disabled mod (user turned off)
INSERT INTO installed_mods (game_id, bottle_name, nexus_mod_id, name, version, archive_name, installed_files, installed_at, enabled, staging_path, install_priority, auto_category, source_type)
VALUES ('skyrimse', 'Test Bottle', 2014, 'Immersive Armors', '8.1', 'ImmersiveArmors.7z', '["Data/Hothtrooper44_ArmorCompilation.esp","Data/Hothtrooper44_ArmorCompilation.bsa"]', '2026-01-20T14:00:00Z', 0, '/tmp/e2e-staging/4', 30, 'Armor', 'nexus');

-- Manual install (no nexus ID)
INSERT INTO installed_mods (game_id, bottle_name, nexus_mod_id, name, version, archive_name, installed_files, installed_at, enabled, staging_path, install_priority, auto_category, source_type, user_notes)
VALUES ('skyrimse', 'Test Bottle', NULL, 'SKSE Scripts', '2.2.6', 'skse64_2_02_06.7z', '["Data/Scripts/Source/ActiveMagicEffect.psc"]', '2026-01-15T10:00:30Z', 1, '/tmp/e2e-staging/5', 0, 'Utilities', 'manual', 'SKSE script files — extract manually');

-- Collection mod (part of a NM collection)
INSERT INTO installed_mods (game_id, bottle_name, nexus_mod_id, name, version, archive_name, installed_files, installed_at, enabled, staging_path, install_priority, collection_name, auto_category, source_type)
VALUES ('skyrimse', 'Test Bottle', 18076, 'JK''s Skyrim', '1.7', 'JKsSkyrim.7z', '["Data/JKs Skyrim.esp","Data/JKs Skyrim.bsa"]', '2026-02-10T09:00:00Z', 1, '/tmp/e2e-staging/6', 50, 'Nordic Souls', 'Cities', 'nexus');

-- Another collection mod (optional, disabled)
INSERT INTO installed_mods (game_id, bottle_name, nexus_mod_id, name, version, archive_name, installed_files, installed_at, enabled, staging_path, install_priority, collection_name, auto_category, source_type, collection_optional)
VALUES ('skyrimse', 'Test Bottle', 74484, 'Lux Orbis', '3.2', 'LuxOrbis.7z', '["Data/Lux - Orbis.esp"]', '2026-02-10T09:01:00Z', 0, '/tmp/e2e-staging/7', 51, 'Nordic Souls', 'Lighting', 'nexus', 1);

-- Mod with user tags and notes
INSERT INTO installed_mods (game_id, bottle_name, nexus_mod_id, name, version, archive_name, installed_files, installed_at, enabled, staging_path, install_priority, auto_category, source_type, user_notes, user_tags)
VALUES ('skyrimse', 'Test Bottle', 3863, 'Static Mesh Improvement Mod', '2.08', 'SMIM.7z', '["Data/SMIM-SE-Merged-All.bsa","Data/SMIM-SE-Merged-All.esp"]', '2026-01-25T16:00:00Z', 1, '/tmp/e2e-staging/8', 20, 'Visuals', 'nexus', 'Performance hit on low-end systems', '["visual","performance-sensitive"]');

-- File conflict between two mods
INSERT INTO file_conflicts (game_id, bottle_name, relative_path, winning_mod_id, losing_mod_ids)
VALUES ('skyrimse', 'Test Bottle', 'Data/meshes/architecture/whiterun/wrterrain01.nif', 6, '[8]');

-- Installed collection
INSERT INTO installed_collections VALUES ('nordic-souls', 'Nordic Souls', 'skyrimspecialedition', 'skyrimse', 'Test Bottle', 'TheModAuthor', 15, 15, 87, 8589934592, 'installed');

-- Depot history (for downgrade testing)
INSERT INTO steam_depot_history (game_id, app_id, depot_id, manifest_id, build_id, game_version)
VALUES ('skyrimse', '489830', '489833', '4063321535627579835', '7940292', '1.5.97');
INSERT INTO steam_depot_history (game_id, app_id, depot_id, manifest_id, build_id, game_version)
VALUES ('skyrimse', '489830', '489833', '8260459358764089032', '14805699', '1.6.1170');

-- Mod dependency: SMIM depends on Address Library
INSERT INTO mod_dependencies (mod_id, depends_on_id, dependency_type) VALUES (8, 1, 'required');

SQL

echo "Fixture DB created at $DB"
echo "Schema version: $(sqlite3 "$DB" 'SELECT version FROM schema_version')"
echo "Mods: $(sqlite3 "$DB" 'SELECT count(*) FROM installed_mods')"
echo "Profiles: $(sqlite3 "$DB" 'SELECT count(*) FROM profiles')"
echo "Conflicts: $(sqlite3 "$DB" 'SELECT count(*) FROM file_conflicts')"
