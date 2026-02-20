//! Game session tracker and stability journal.
//!
//! Records game launch/exit events, correlates mod changes with sessions,
//! and builds a stability timeline for debugging crash-prone configurations.

use serde::Serialize;

use crate::database::ModDatabase;

/// A recorded game session.
#[derive(Clone, Debug, Serialize)]
pub struct GameSession {
    pub id: i64,
    pub game_id: String,
    pub bottle_name: String,
    pub profile_name: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub duration_secs: Option<i64>,
    pub clean_exit: Option<bool>,
    pub crash_log_path: Option<String>,
    pub notes: Option<String>,
}

/// A mod change associated with a session.
#[derive(Clone, Debug, Serialize)]
pub struct SessionModChange {
    pub id: i64,
    pub session_id: i64,
    pub mod_id: Option<i64>,
    pub mod_name: String,
    pub change_type: String,
    pub detail: Option<String>,
}

/// Stability summary for a game.
#[derive(Clone, Debug, Serialize)]
pub struct StabilitySummary {
    pub total_sessions: usize,
    pub clean_exits: usize,
    pub crashes: usize,
    pub unknown_exits: usize,
    pub avg_duration_secs: f64,
    pub last_stable_session: Option<String>,
    pub mods_since_last_stable: Vec<SessionModChange>,
}

/// Start a new game session.
pub fn start_session(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    profile_name: Option<&str>,
) -> Result<i64, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().to_rfc3339();

    conn.execute(
        "INSERT INTO game_sessions (game_id, bottle_name, profile_name, started_at)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![game_id, bottle_name, profile_name, now],
    )
    .map_err(|e| e.to_string())?;

    Ok(conn.last_insert_rowid())
}

/// End a game session.
pub fn end_session(
    db: &ModDatabase,
    session_id: i64,
    clean_exit: bool,
    crash_log_path: Option<&str>,
) -> Result<(), String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().to_rfc3339();

    // Calculate duration
    let started_at: String = conn
        .prepare("SELECT started_at FROM game_sessions WHERE id = ?1")
        .map_err(|e| e.to_string())?
        .query_row([session_id], |row| row.get(0))
        .map_err(|e| e.to_string())?;

    let duration = chrono::DateTime::parse_from_rfc3339(&now)
        .ok()
        .and_then(|end| {
            chrono::DateTime::parse_from_rfc3339(&started_at)
                .ok()
                .map(|start| (end - start).num_seconds())
        });

    conn.execute(
        "UPDATE game_sessions
         SET ended_at = ?1, duration_secs = ?2, clean_exit = ?3, crash_log_path = ?4
         WHERE id = ?5",
        rusqlite::params![now, duration, clean_exit as i64, crash_log_path, session_id],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

/// Record a mod change for a session.
pub fn record_mod_change(
    db: &ModDatabase,
    session_id: i64,
    mod_id: Option<i64>,
    mod_name: &str,
    change_type: &str,
    detail: Option<&str>,
) -> Result<i64, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;

    conn.execute(
        "INSERT INTO session_mod_changes (session_id, mod_id, mod_name, change_type, detail)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![session_id, mod_id, mod_name, change_type, detail],
    )
    .map_err(|e| e.to_string())?;

    Ok(conn.last_insert_rowid())
}

/// Get session history for a game.
pub fn get_session_history(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    limit: usize,
) -> Result<Vec<GameSession>, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, game_id, bottle_name, profile_name, started_at, ended_at,
                    duration_secs, clean_exit, crash_log_path, notes
             FROM game_sessions
             WHERE game_id = ?1 AND bottle_name = ?2
             ORDER BY started_at DESC
             LIMIT ?3",
        )
        .map_err(|e| e.to_string())?;

    let sessions = stmt
        .query_map(
            rusqlite::params![game_id, bottle_name, limit as i64],
            |row| {
                Ok(GameSession {
                    id: row.get(0)?,
                    game_id: row.get(1)?,
                    bottle_name: row.get(2)?,
                    profile_name: row.get(3)?,
                    started_at: row.get(4)?,
                    ended_at: row.get(5)?,
                    duration_secs: row.get(6)?,
                    clean_exit: row.get::<_, Option<i64>>(7)?.map(|v| v != 0),
                    crash_log_path: row.get(8)?,
                    notes: row.get(9)?,
                })
            },
        )
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    Ok(sessions)
}

/// Get mod changes for a specific session.
pub fn get_session_changes(
    db: &ModDatabase,
    session_id: i64,
) -> Result<Vec<SessionModChange>, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, session_id, mod_id, mod_name, change_type, detail
             FROM session_mod_changes
             WHERE session_id = ?1",
        )
        .map_err(|e| e.to_string())?;

    let changes = stmt
        .query_map([session_id], |row| {
            Ok(SessionModChange {
                id: row.get(0)?,
                session_id: row.get(1)?,
                mod_id: row.get(2)?,
                mod_name: row.get(3)?,
                change_type: row.get(4)?,
                detail: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    Ok(changes)
}

/// Get stability summary for a game.
pub fn get_stability_summary(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
) -> Result<StabilitySummary, String> {
    let sessions = get_session_history(db, game_id, bottle_name, 100)?;

    let total = sessions.len();
    let clean = sessions
        .iter()
        .filter(|s| s.clean_exit == Some(true))
        .count();
    let crashes = sessions
        .iter()
        .filter(|s| s.clean_exit == Some(false))
        .count();
    let unknown = total - clean - crashes;

    let durations: Vec<f64> = sessions
        .iter()
        .filter_map(|s| s.duration_secs.map(|d| d as f64))
        .collect();

    let avg_duration = if durations.is_empty() {
        0.0
    } else {
        durations.iter().sum::<f64>() / durations.len() as f64
    };

    // Find the last stable (clean exit) session
    let last_stable = sessions
        .iter()
        .find(|s| s.clean_exit == Some(true))
        .map(|s| s.started_at.clone());

    // Get mod changes since the last stable session
    let mods_since_stable = if let Some(ref stable_at) = last_stable {
        let conn = db.conn().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT smc.id, smc.session_id, smc.mod_id, smc.mod_name, smc.change_type, smc.detail
                 FROM session_mod_changes smc
                 JOIN game_sessions gs ON gs.id = smc.session_id
                 WHERE gs.game_id = ?1 AND gs.bottle_name = ?2 AND gs.started_at > ?3
                 ORDER BY gs.started_at DESC",
            )
            .map_err(|e| e.to_string())?;

        let results: Vec<SessionModChange> = stmt
            .query_map(rusqlite::params![game_id, bottle_name, stable_at], |row| {
                Ok(SessionModChange {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    mod_id: row.get(2)?,
                    mod_name: row.get(3)?,
                    change_type: row.get(4)?,
                    detail: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        results
    } else {
        Vec::new()
    };

    Ok(StabilitySummary {
        total_sessions: total,
        clean_exits: clean,
        crashes,
        unknown_exits: unknown,
        avg_duration_secs: avg_duration,
        last_stable_session: last_stable,
        mods_since_last_stable: mods_since_stable,
    })
}

/// Add a note to a session.
pub fn set_session_notes(db: &ModDatabase, session_id: i64, notes: &str) -> Result<(), String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE game_sessions SET notes = ?1 WHERE id = ?2",
        rusqlite::params![notes, session_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Delete old sessions beyond a limit.
pub fn cleanup_old_sessions(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    keep_count: usize,
) -> Result<usize, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;

    let deleted = conn
        .execute(
            "DELETE FROM game_sessions
             WHERE id NOT IN (
                 SELECT id FROM game_sessions
                 WHERE game_id = ?1 AND bottle_name = ?2
                 ORDER BY started_at DESC
                 LIMIT ?3
             ) AND game_id = ?1 AND bottle_name = ?2",
            rusqlite::params![game_id, bottle_name, keep_count as i64],
        )
        .map_err(|e| e.to_string())?;

    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> ModDatabase {
        ModDatabase::new(std::path::Path::new(":memory:")).unwrap()
    }

    #[test]
    fn start_session_returns_id() {
        let db = test_db();
        let id = start_session(&db, "skyrimse", "Gaming", None).unwrap();
        assert!(id > 0);
    }

    #[test]
    fn start_session_unique_ids() {
        let db = test_db();
        let id1 = start_session(&db, "skyrimse", "Gaming", None).unwrap();
        let id2 = start_session(&db, "skyrimse", "Gaming", None).unwrap();
        assert_ne!(id1, id2);
    }

    #[test]
    fn start_session_with_profile() {
        let db = test_db();
        let id = start_session(&db, "skyrimse", "Gaming", Some("Default")).unwrap();
        let sessions = get_session_history(&db, "skyrimse", "Gaming", 10).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, id);
        assert_eq!(sessions[0].profile_name, Some("Default".to_string()));
    }

    #[test]
    fn start_session_stores_started_at() {
        let db = test_db();
        start_session(&db, "skyrimse", "Gaming", None).unwrap();
        let sessions = get_session_history(&db, "skyrimse", "Gaming", 10).unwrap();
        assert!(!sessions[0].started_at.is_empty());
    }

    #[test]
    fn end_session_records_clean_exit() {
        let db = test_db();
        let id = start_session(&db, "skyrimse", "Gaming", None).unwrap();
        end_session(&db, id, true, None).unwrap();

        let sessions = get_session_history(&db, "skyrimse", "Gaming", 10).unwrap();
        assert_eq!(sessions[0].clean_exit, Some(true));
        assert!(sessions[0].ended_at.is_some());
    }

    #[test]
    fn end_session_records_crash() {
        let db = test_db();
        let id = start_session(&db, "skyrimse", "Gaming", None).unwrap();
        end_session(&db, id, false, Some("/path/to/crash.log")).unwrap();

        let sessions = get_session_history(&db, "skyrimse", "Gaming", 10).unwrap();
        assert_eq!(sessions[0].clean_exit, Some(false));
        assert_eq!(
            sessions[0].crash_log_path,
            Some("/path/to/crash.log".to_string())
        );
    }

    #[test]
    fn end_session_calculates_duration() {
        let db = test_db();
        let id = start_session(&db, "skyrimse", "Gaming", None).unwrap();
        end_session(&db, id, true, None).unwrap();

        let sessions = get_session_history(&db, "skyrimse", "Gaming", 10).unwrap();
        assert!(sessions[0].duration_secs.is_some());
        assert!(sessions[0].duration_secs.unwrap() >= 0);
    }

    #[test]
    fn end_session_without_crash_log() {
        let db = test_db();
        let id = start_session(&db, "skyrimse", "Gaming", None).unwrap();
        end_session(&db, id, true, None).unwrap();

        let sessions = get_session_history(&db, "skyrimse", "Gaming", 10).unwrap();
        assert!(sessions[0].crash_log_path.is_none());
    }

    #[test]
    fn record_mod_change_works() {
        let db = test_db();
        let session_id = start_session(&db, "skyrimse", "Gaming", None).unwrap();

        let change_id =
            record_mod_change(&db, session_id, Some(1), "SkyUI", "added", None).unwrap();
        assert!(change_id > 0);

        let changes = get_session_changes(&db, session_id).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].mod_name, "SkyUI");
        assert_eq!(changes[0].change_type, "added");
    }

    #[test]
    fn record_mod_change_unique_ids() {
        let db = test_db();
        let sid = start_session(&db, "skyrimse", "Gaming", None).unwrap();
        let id1 = record_mod_change(&db, sid, Some(1), "Mod A", "added", None).unwrap();
        let id2 = record_mod_change(&db, sid, Some(2), "Mod B", "added", None).unwrap();
        assert_ne!(id1, id2);
    }

    #[test]
    fn record_mod_change_with_detail() {
        let db = test_db();
        let sid = start_session(&db, "skyrimse", "Gaming", None).unwrap();
        record_mod_change(&db, sid, Some(1), "Mod A", "updated", Some("1.0 -> 2.0")).unwrap();

        let changes = get_session_changes(&db, sid).unwrap();
        assert_eq!(changes[0].detail, Some("1.0 -> 2.0".to_string()));
    }

    #[test]
    fn record_mod_change_without_mod_id() {
        let db = test_db();
        let sid = start_session(&db, "skyrimse", "Gaming", None).unwrap();
        record_mod_change(&db, sid, None, "External Mod", "removed", None).unwrap();

        let changes = get_session_changes(&db, sid).unwrap();
        assert!(changes[0].mod_id.is_none());
    }

    #[test]
    fn get_session_history_ordered_newest_first() {
        let db = test_db();
        start_session(&db, "skyrimse", "Gaming", None).unwrap();
        start_session(&db, "skyrimse", "Gaming", None).unwrap();

        let sessions = get_session_history(&db, "skyrimse", "Gaming", 10).unwrap();
        assert_eq!(sessions.len(), 2);
        assert!(sessions[0].started_at >= sessions[1].started_at);
    }

    #[test]
    fn get_session_history_respects_limit() {
        let db = test_db();
        for _ in 0..5 {
            start_session(&db, "skyrimse", "Gaming", None).unwrap();
        }

        let sessions = get_session_history(&db, "skyrimse", "Gaming", 3).unwrap();
        assert_eq!(sessions.len(), 3);
    }

    #[test]
    fn get_session_history_filters_by_game() {
        let db = test_db();
        start_session(&db, "skyrimse", "Gaming", None).unwrap();
        start_session(&db, "fallout4", "Gaming", None).unwrap();

        let sessions = get_session_history(&db, "skyrimse", "Gaming", 10).unwrap();
        assert_eq!(sessions.len(), 1);
    }

    #[test]
    fn get_session_history_empty() {
        let db = test_db();
        let sessions = get_session_history(&db, "skyrimse", "Gaming", 10).unwrap();
        assert!(sessions.is_empty());
    }

    #[test]
    fn get_session_changes_empty_session() {
        let db = test_db();
        let sid = start_session(&db, "skyrimse", "Gaming", None).unwrap();
        let changes = get_session_changes(&db, sid).unwrap();
        assert!(changes.is_empty());
    }

    #[test]
    fn get_session_changes_nonexistent_session() {
        let db = test_db();
        let changes = get_session_changes(&db, 99999).unwrap();
        assert!(changes.is_empty());
    }

    #[test]
    fn stability_summary_empty() {
        let db = test_db();
        let summary = get_stability_summary(&db, "skyrimse", "Gaming").unwrap();
        assert_eq!(summary.total_sessions, 0);
        assert_eq!(summary.clean_exits, 0);
        assert_eq!(summary.crashes, 0);
    }

    #[test]
    fn stability_summary_all_clean() {
        let db = test_db();
        for _ in 0..3 {
            let id = start_session(&db, "skyrimse", "Gaming", None).unwrap();
            end_session(&db, id, true, None).unwrap();
        }

        let summary = get_stability_summary(&db, "skyrimse", "Gaming").unwrap();
        assert_eq!(summary.total_sessions, 3);
        assert_eq!(summary.clean_exits, 3);
        assert_eq!(summary.crashes, 0);
    }

    #[test]
    fn stability_summary_mixed() {
        let db = test_db();
        let id1 = start_session(&db, "skyrimse", "Gaming", None).unwrap();
        end_session(&db, id1, true, None).unwrap();
        let id2 = start_session(&db, "skyrimse", "Gaming", None).unwrap();
        end_session(&db, id2, false, None).unwrap();

        let summary = get_stability_summary(&db, "skyrimse", "Gaming").unwrap();
        assert_eq!(summary.total_sessions, 2);
        assert_eq!(summary.clean_exits, 1);
        assert_eq!(summary.crashes, 1);
    }

    #[test]
    fn stability_summary_avg_duration() {
        let db = test_db();
        let id = start_session(&db, "skyrimse", "Gaming", None).unwrap();
        end_session(&db, id, true, None).unwrap();

        let summary = get_stability_summary(&db, "skyrimse", "Gaming").unwrap();
        assert!(summary.avg_duration_secs >= 0.0);
    }

    #[test]
    fn set_session_notes_works() {
        let db = test_db();
        let id = start_session(&db, "skyrimse", "Gaming", None).unwrap();
        set_session_notes(&db, id, "Test note").unwrap();

        let sessions = get_session_history(&db, "skyrimse", "Gaming", 10).unwrap();
        assert_eq!(sessions[0].notes, Some("Test note".to_string()));
    }

    #[test]
    fn set_session_notes_overwrite() {
        let db = test_db();
        let id = start_session(&db, "skyrimse", "Gaming", None).unwrap();
        set_session_notes(&db, id, "First").unwrap();
        set_session_notes(&db, id, "Second").unwrap();

        let sessions = get_session_history(&db, "skyrimse", "Gaming", 10).unwrap();
        assert_eq!(sessions[0].notes, Some("Second".to_string()));
    }

    #[test]
    fn cleanup_old_sessions_preserves_recent() {
        let db = test_db();
        for _ in 0..5 {
            start_session(&db, "skyrimse", "Gaming", None).unwrap();
        }

        let deleted = cleanup_old_sessions(&db, "skyrimse", "Gaming", 3).unwrap();
        assert_eq!(deleted, 2);

        let remaining = get_session_history(&db, "skyrimse", "Gaming", 100).unwrap();
        assert_eq!(remaining.len(), 3);
    }

    #[test]
    fn cleanup_old_sessions_nothing_to_clean() {
        let db = test_db();
        for _ in 0..2 {
            start_session(&db, "skyrimse", "Gaming", None).unwrap();
        }

        let deleted = cleanup_old_sessions(&db, "skyrimse", "Gaming", 10).unwrap();
        assert_eq!(deleted, 0);
    }

    #[test]
    fn cleanup_old_sessions_empty() {
        let db = test_db();
        let deleted = cleanup_old_sessions(&db, "skyrimse", "Gaming", 5).unwrap();
        assert_eq!(deleted, 0);
    }

    #[test]
    fn cleanup_preserves_other_games() {
        let db = test_db();
        start_session(&db, "skyrimse", "Gaming", None).unwrap();
        start_session(&db, "fallout4", "Gaming", None).unwrap();

        cleanup_old_sessions(&db, "skyrimse", "Gaming", 0).unwrap();

        let fo4 = get_session_history(&db, "fallout4", "Gaming", 10).unwrap();
        assert_eq!(fo4.len(), 1);
    }

    #[test]
    fn get_session_changes_returns_changes() {
        let db = test_db();
        let sid = start_session(&db, "skyrimse", "Gaming", None).unwrap();
        record_mod_change(&db, sid, Some(42), "USSEP", "added", Some("v4.2.8")).unwrap();

        let changes = get_session_changes(&db, sid).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].session_id, sid);
    }

    #[test]
    fn get_session_changes_correct_fields() {
        let db = test_db();
        let sid = start_session(&db, "skyrimse", "Gaming", None).unwrap();
        record_mod_change(
            &db,
            sid,
            Some(10),
            "RaceMenu",
            "updated",
            Some("0.4.19 -> 0.4.20"),
        )
        .unwrap();

        let changes = get_session_changes(&db, sid).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].change_type, "updated");
        assert_eq!(changes[0].mod_name, "RaceMenu");
        assert_eq!(changes[0].mod_id, Some(10));
        assert_eq!(changes[0].detail, Some("0.4.19 -> 0.4.20".to_string()));
    }

    #[test]
    fn set_session_notes_empty_string() {
        let db = test_db();
        let id = start_session(&db, "skyrimse", "Gaming", None).unwrap();
        set_session_notes(&db, id, "Some note first").unwrap();
        set_session_notes(&db, id, "").unwrap();

        let sessions = get_session_history(&db, "skyrimse", "Gaming", 10).unwrap();
        assert_eq!(sessions[0].notes, Some("".to_string()));
    }

    #[test]
    fn set_session_notes_long_text() {
        let db = test_db();
        let id = start_session(&db, "skyrimse", "Gaming", None).unwrap();
        let long_note = "A".repeat(10_000);
        set_session_notes(&db, id, &long_note).unwrap();

        let sessions = get_session_history(&db, "skyrimse", "Gaming", 10).unwrap();
        assert_eq!(sessions[0].notes, Some(long_note));
    }
}
