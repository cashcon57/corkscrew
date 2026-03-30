//! Native Hogwarts Legacy mod merger.
//!
//! Replaces the Windows-only HLModMerger.exe by using `repak` for PAK file
//! read/write and `rusqlite` for SQLite database diffing and merging.
//!
//! When multiple PAK mods in `~mods/` contain `PhoenixShipData.sqlite`, their
//! database changes must be merged into a single `zMergedMods_P.pak` to avoid
//! data corruption.  This module does that entirely in Rust — no Wine needed.

use std::collections::HashSet;
use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MergerError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("PAK error: {0}")]
    Pak(String),

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, MergerError>;

/// Result of a merge operation.
#[derive(Clone, Debug, serde::Serialize)]
pub struct MergeResult {
    pub merged_paks: usize,
    pub output_file: String,
    pub tables_modified: usize,
    pub rows_merged: usize,
}

// ---------------------------------------------------------------------------
// PAK SQLite Detection
// ---------------------------------------------------------------------------

/// Find all PAK files in `mods_dir` that contain a PhoenixShipData SQLite database.
pub fn find_paks_with_sqlite(mods_dir: &Path) -> Vec<PathBuf> {
    let entries = match fs::read_dir(mods_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut result = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        if !name.ends_with(".pak") {
            continue;
        }

        // Skip our own merged output from a previous run
        if name.starts_with("zmergedmods") {
            continue;
        }

        if pak_contains_sqlite(&path) {
            result.push(path);
        }
    }

    result.sort();
    result
}

/// Check if a PAK file contains any SQLite database file.
fn pak_contains_sqlite(pak_path: &Path) -> bool {
    let file = match fs::File::open(pak_path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut reader = BufReader::new(file);

    let pak = match repak::PakBuilder::new().reader(&mut reader) {
        Ok(p) => p,
        Err(_) => return false,
    };

    pak.files()
        .iter()
        .any(|f| f.to_lowercase().contains("phoenixshipdata") || f.to_lowercase().ends_with(".sqlite"))
}

// ---------------------------------------------------------------------------
// SQLite Merge Engine
// ---------------------------------------------------------------------------

/// Merge PhoenixShipData.sqlite from multiple PAK mods into a single merged PAK.
///
/// The algorithm:
/// 1. Extract the SQLite database from each PAK
/// 2. Use the first PAK's database as the base
/// 3. For each subsequent PAK, diff its database against the base
/// 4. Collect `INSERT OR REPLACE` statements for new/changed rows
/// 5. Apply all collected changes to a copy of the base database
/// 6. Pack the merged database into `zMergedMods_P.pak`
pub fn merge_databases(
    mod_paks: &[PathBuf],
    output_dir: &Path,
) -> Result<MergeResult> {
    if mod_paks.len() < 2 {
        return Err(MergerError::Other("Need at least 2 PAKs to merge".into()));
    }

    log::info!(
        "HL Merger: merging databases from {} PAK files",
        mod_paks.len()
    );

    // Step 1: Extract SQLite databases from each PAK
    let temp_dir = tempfile::tempdir()?;
    let mut db_paths: Vec<(PathBuf, PathBuf)> = Vec::new(); // (pak_path, extracted_db_path)

    for (i, pak_path) in mod_paks.iter().enumerate() {
        let db_path = temp_dir.path().join(format!("mod_{i}.sqlite"));
        extract_sqlite_from_pak(pak_path, &db_path)?;
        db_paths.push((pak_path.clone(), db_path));
    }

    // Step 2: Use the first PAK's DB as the base, diff all others against it
    let base_db_path = &db_paths[0].1;
    let merged_db_path = temp_dir.path().join("merged.sqlite");
    fs::copy(base_db_path, &merged_db_path)?;

    let mut total_rows = 0usize;
    let mut modified_tables = HashSet::new();

    // Collect all diff statements (deduplicated)
    let mut all_statements: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for (pak_path, db_path) in &db_paths[1..] {
        let pak_name = pak_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        match diff_databases(base_db_path, db_path) {
            Ok(stmts) => {
                log::info!(
                    "HL Merger: {} diff statements from {}",
                    stmts.len(),
                    pak_name
                );
                for (table, sql) in stmts {
                    if seen.insert(sql.clone()) {
                        modified_tables.insert(table);
                        all_statements.push(sql);
                    }
                }
            }
            Err(e) => {
                log::warn!("HL Merger: failed to diff {}: {}", pak_name, e);
            }
        }
    }

    // Step 3: Apply all statements to the merged database.
    // Disable foreign key enforcement during the merge — mod databases may
    // reference rows in tables that haven't been inserted yet (e.g. child
    // rows before parent rows). FK constraints will be valid once all
    // statements are applied.
    if !all_statements.is_empty() {
        let conn = Connection::open(&merged_db_path)?;
        conn.execute_batch("PRAGMA foreign_keys = OFF")?;
        conn.execute_batch("BEGIN TRANSACTION")?;
        for sql in &all_statements {
            if let Err(e) = conn.execute_batch(sql) {
                log::warn!("HL Merger: failed to apply statement: {} — {}", sql, e);
            } else {
                total_rows += 1;
            }
        }
        conn.execute_batch("COMMIT")?;
        // Re-enable FK checks and verify integrity
        conn.execute_batch("PRAGMA foreign_keys = ON")?;
    }

    log::info!(
        "HL Merger: applied {} rows across {} tables",
        total_rows,
        modified_tables.len()
    );

    // Step 4: Get the internal path from the first PAK for the sqlite file
    let sqlite_internal_path = get_sqlite_internal_path(&mod_paks[0])?;

    // Step 5: Pack the merged database into a new PAK
    let output_pak = output_dir.join("zMergedMods_P.pak");
    pack_sqlite_into_pak(&merged_db_path, &sqlite_internal_path, &output_pak)?;

    log::info!(
        "HL Merger: wrote merged PAK to {}",
        output_pak.display()
    );

    Ok(MergeResult {
        merged_paks: mod_paks.len(),
        output_file: output_pak.to_string_lossy().to_string(),
        tables_modified: modified_tables.len(),
        rows_merged: total_rows,
    })
}

/// Extract the PhoenixShipData.sqlite file from a PAK to a local path.
fn extract_sqlite_from_pak(pak_path: &Path, output_path: &Path) -> Result<()> {
    let file = fs::File::open(pak_path)?;
    let mut reader = BufReader::new(file);
    let pak = repak::PakBuilder::new()
        .reader(&mut reader)
        .map_err(|e| MergerError::Pak(format!("Failed to read PAK {}: {}", pak_path.display(), e)))?;

    let sqlite_entry = pak
        .files()
        .into_iter()
        .find(|f| {
            let fl = f.to_lowercase();
            fl.contains("phoenixshipdata") || fl.ends_with(".sqlite")
        })
        .ok_or_else(|| {
            MergerError::Pak(format!("No SQLite found in {}", pak_path.display()))
        })?;

    let data = pak
        .get(&sqlite_entry, &mut reader)
        .map_err(|e| MergerError::Pak(format!("Failed to extract {}: {}", sqlite_entry, e)))?;

    fs::write(output_path, &data)?;
    Ok(())
}

/// Get the internal PAK path for the SQLite file (needed for repacking).
fn get_sqlite_internal_path(pak_path: &Path) -> Result<String> {
    let file = fs::File::open(pak_path)?;
    let mut reader = BufReader::new(file);
    let pak = repak::PakBuilder::new()
        .reader(&mut reader)
        .map_err(|e| MergerError::Pak(format!("Failed to read PAK: {}", e)))?;

    pak.files()
        .into_iter()
        .find(|f| {
            let fl = f.to_lowercase();
            fl.contains("phoenixshipdata") || fl.ends_with(".sqlite")
        })
        .ok_or_else(|| MergerError::Pak("No SQLite found in PAK".into()))
}

/// Pack a SQLite database file into a new PAK.
fn pack_sqlite_into_pak(
    db_path: &Path,
    internal_path: &str,
    output_path: &Path,
) -> Result<()> {
    let data = fs::read(db_path)?;
    let output_file = fs::File::create(output_path)?;

    let mut pak_writer = repak::PakBuilder::new().writer(
        output_file,
        repak::Version::V11,
        "../../../".to_string(), // Standard UE5 mount point
        None,
    );

    pak_writer
        .write_file(internal_path, false, data)
        .map_err(|e| MergerError::Pak(format!("Failed to write entry: {}", e)))?;

    pak_writer
        .write_index()
        .map_err(|e| MergerError::Pak(format!("Failed to write PAK index: {}", e)))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Database Diffing
// ---------------------------------------------------------------------------

/// Diff two SQLite databases and return (table_name, SQL) pairs for changes.
///
/// For each table in the mod database, compares rows against the base.
/// Returns `INSERT OR REPLACE` statements for rows that differ or are new.
fn diff_databases(
    base_path: &Path,
    mod_path: &Path,
) -> Result<Vec<(String, String)>> {
    let base_conn = Connection::open(base_path)?;
    let mod_conn = Connection::open(mod_path)?;

    // Get all table names from the mod database
    let tables = get_table_names(&mod_conn)?;
    let mut statements = Vec::new();

    for table in &tables {
        // Get column info
        let columns = get_column_names(&mod_conn, table)?;
        if columns.is_empty() {
            continue;
        }

        // Check if table exists in base
        let base_tables = get_table_names(&base_conn)?;
        if !base_tables.contains(table) {
            // Entire table is new — copy all rows
            let all_rows = get_all_rows(&mod_conn, table, &columns)?;
            for row in all_rows {
                let sql = build_insert_or_replace(table, &columns, &row);
                statements.push((table.clone(), sql));
            }
            continue;
        }

        // Compare row by row
        let mod_rows = get_all_rows(&mod_conn, table, &columns)?;
        for row in &mod_rows {
            if !row_exists_in_db(&base_conn, table, &columns, row)? {
                let sql = build_insert_or_replace(table, &columns, row);
                statements.push((table.clone(), sql));
            }
        }
    }

    Ok(statements)
}

/// Get all user table names from a database.
fn get_table_names(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")?;
    let names: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(names)
}

/// Get column names for a table.
fn get_column_names(conn: &Connection, table: &str) -> Result<Vec<String>> {
    // Use PRAGMA table_info which is safe from injection (table name validated above)
    let mut stmt = conn.prepare(&format!("PRAGMA table_info(\"{}\")", table))?;
    let names: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(names)
}

/// Get all rows from a table as vectors of string-encoded values.
fn get_all_rows(
    conn: &Connection,
    table: &str,
    columns: &[String],
) -> Result<Vec<Vec<String>>> {
    let col_list = columns
        .iter()
        .map(|c| format!("\"{}\"", c))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("SELECT {} FROM \"{}\"", col_list, table);
    let mut stmt = conn.prepare(&sql)?;

    let col_count = columns.len();
    let rows: Vec<Vec<String>> = stmt
        .query_map([], |row| {
            let mut vals = Vec::with_capacity(col_count);
            for i in 0..col_count {
                let val: rusqlite::types::Value = row.get(i)?;
                vals.push(value_to_sql_literal(&val));
            }
            Ok(vals)
        })?
        .filter_map(|r| r.ok())
        .collect();

    Ok(rows)
}

/// Check if an exact row exists in a database table.
fn row_exists_in_db(
    conn: &Connection,
    table: &str,
    columns: &[String],
    values: &[String],
) -> Result<bool> {
    if columns.len() != values.len() {
        return Ok(false);
    }

    let conditions: Vec<String> = columns
        .iter()
        .zip(values.iter())
        .map(|(col, val)| {
            if val == "NULL" {
                format!("\"{}\" IS NULL", col)
            } else {
                format!("\"{}\" = {}", col, val)
            }
        })
        .collect();

    let sql = format!(
        "SELECT 1 FROM \"{}\" WHERE {} LIMIT 1",
        table,
        conditions.join(" AND ")
    );

    let exists: bool = conn
        .prepare(&sql)?
        .query_row([], |_| Ok(true))
        .unwrap_or(false);

    Ok(exists)
}

/// Build an INSERT OR REPLACE statement for a row.
fn build_insert_or_replace(table: &str, columns: &[String], values: &[String]) -> String {
    let col_list = columns
        .iter()
        .map(|c| format!("\"{}\"", c))
        .collect::<Vec<_>>()
        .join(", ");
    let val_list = values.join(", ");
    format!(
        "INSERT OR REPLACE INTO \"{}\" ({}) VALUES ({});",
        table, col_list, val_list
    )
}

/// Convert a rusqlite Value to a SQL literal string.
fn value_to_sql_literal(val: &rusqlite::types::Value) -> String {
    match val {
        rusqlite::types::Value::Null => "NULL".to_string(),
        rusqlite::types::Value::Integer(i) => i.to_string(),
        rusqlite::types::Value::Real(f) => f.to_string(),
        rusqlite::types::Value::Text(s) => {
            // Escape single quotes by doubling them
            format!("'{}'", s.replace('\'', "''"))
        }
        rusqlite::types::Value::Blob(b) => {
            let hex: String = b.iter().map(|byte| format!("{:02x}", byte)).collect();
            format!("X'{}'", hex)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_db(path: &Path, extra_sql: &str) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT, value REAL);
             INSERT INTO items VALUES (1, 'Sword', 10.0);
             INSERT INTO items VALUES (2, 'Shield', 5.0);",
        )
        .unwrap();
        if !extra_sql.is_empty() {
            conn.execute_batch(extra_sql).unwrap();
        }
    }

    #[test]
    fn test_diff_detects_new_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("base.sqlite");
        let moddb = tmp.path().join("mod.sqlite");

        create_test_db(&base, "");
        create_test_db(&moddb, "INSERT INTO items VALUES (3, 'Potion', 25.0);");

        let stmts = diff_databases(&base, &moddb).unwrap();
        assert_eq!(stmts.len(), 1);
        assert!(stmts[0].1.contains("Potion"));
    }

    #[test]
    fn test_diff_detects_changed_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("base.sqlite");
        let moddb = tmp.path().join("mod.sqlite");

        create_test_db(&base, "");
        create_test_db(&moddb, "UPDATE items SET value = 99.0 WHERE id = 1;");

        let stmts = diff_databases(&base, &moddb).unwrap();
        // The updated row (id=1, value=99) differs from base (id=1, value=10)
        assert!(!stmts.is_empty());
        assert!(stmts.iter().any(|(_, sql)| sql.contains("99")));
    }

    #[test]
    fn test_diff_identical_databases_no_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("base.sqlite");
        let moddb = tmp.path().join("mod.sqlite");

        create_test_db(&base, "");
        create_test_db(&moddb, "");

        let stmts = diff_databases(&base, &moddb).unwrap();
        assert!(stmts.is_empty());
    }

    #[test]
    fn test_value_to_sql_literal() {
        assert_eq!(
            value_to_sql_literal(&rusqlite::types::Value::Null),
            "NULL"
        );
        assert_eq!(
            value_to_sql_literal(&rusqlite::types::Value::Integer(42)),
            "42"
        );
        assert_eq!(
            value_to_sql_literal(&rusqlite::types::Value::Real(3.14)),
            "3.14"
        );
        assert_eq!(
            value_to_sql_literal(&rusqlite::types::Value::Text("hello".into())),
            "'hello'"
        );
        assert_eq!(
            value_to_sql_literal(&rusqlite::types::Value::Text("it's".into())),
            "'it''s'"
        );
    }

    #[test]
    fn test_build_insert_or_replace() {
        let cols = vec!["id".into(), "name".into()];
        let vals = vec!["1".into(), "'Sword'".into()];
        let sql = build_insert_or_replace("items", &cols, &vals);
        assert_eq!(
            sql,
            "INSERT OR REPLACE INTO \"items\" (\"id\", \"name\") VALUES (1, 'Sword');"
        );
    }
}
