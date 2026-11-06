//! The project database: schema, migrations, connection settings.
//!
//! The schema version is SQLite's `user_version`; migrations run in order
//! when a project opens, in one transaction that holds the write lock. A database newer
//! than this engine is refused rather than misread.

use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;

use super::runs::job_paths;
use crate::error::{Error, Result};

/// A schema change: SQL, then an optional data step in Rust.
struct Migration {
    sql: &'static str,
    then: Option<fn(&rusqlite::Transaction<'_>) -> rusqlite::Result<()>>,
}

/// Migrations, in order; migration `i` brings the schema to version `i + 1`.
const MIGRATIONS: &[Migration] = &[
    // 1: project, runs, datasets (with the run that wrote them), saved jobs.
    Migration {
        sql: V1,
        then: None,
    },
    // 2: the datasets each run read, recovered for earlier runs from their jobs.
    Migration {
        sql: "CREATE TABLE run_inputs (
            run INTEGER NOT NULL REFERENCES runs (id) ON DELETE CASCADE,
            dataset INTEGER NOT NULL REFERENCES datasets (id) ON DELETE CASCADE,
            PRIMARY KEY (run, dataset)
        );
        CREATE INDEX run_inputs_dataset ON run_inputs (dataset);",
        then: Some(backfill_run_inputs),
    },
    // 3: runs allowed to replace datasets of their classes and tags.
    Migration {
        sql: "ALTER TABLE runs ADD COLUMN overwrite INTEGER NOT NULL DEFAULT 0;",
        then: None,
    },
];

const V1: &str = "CREATE TABLE meta (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    CREATE TABLE runs (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        analysis TEXT NOT NULL,
        tags TEXT NOT NULL,
        job TEXT NOT NULL,
        status TEXT NOT NULL
            CHECK (status IN ('queued', 'running', 'done', 'failed', 'cancelled', 'interrupted')),
        output_dir TEXT NOT NULL,
        outcome TEXT,
        error TEXT,
        started INTEGER NOT NULL,
        finished INTEGER
    );
    CREATE INDEX runs_status ON runs (status);
    CREATE TABLE datasets (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        class TEXT NOT NULL,
        tags TEXT NOT NULL,
        tags_key TEXT NOT NULL,
        file TEXT NOT NULL,
        state TEXT NOT NULL CHECK (state IN ('importing', 'ready', 'deleting')),
        origin TEXT NOT NULL,
        run INTEGER REFERENCES runs (id),
        sha256 TEXT,
        bytes INTEGER,
        preview TEXT,
        created INTEGER NOT NULL
    );
    CREATE UNIQUE INDEX datasets_class_tags ON datasets (class, tags_key);
    CREATE INDEX datasets_run ON datasets (run);
    CREATE INDEX datasets_file ON datasets (file);
    CREATE TABLE jobs (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        analysis TEXT NOT NULL,
        name TEXT NOT NULL,
        document TEXT NOT NULL,
        revision INTEGER NOT NULL DEFAULT 1,
        updated INTEGER NOT NULL
    );";

/// Link every recorded run to the ready datasets its job reads: the paths
/// of its typed job, as `start_run` records them.
fn backfill_run_inputs(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    let runs: Vec<(i64, String, String)> = {
        let mut stmt = tx.prepare("SELECT id, analysis, job FROM runs")?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?
    };
    let mut find = tx
        .prepare("SELECT id FROM datasets WHERE file = ?1 AND state = 'ready' AND run IS NOT ?2")?;
    let mut link = tx.prepare("INSERT OR IGNORE INTO run_inputs (run, dataset) VALUES (?1, ?2)")?;
    for (run, analysis, job) in runs {
        for key in job_paths(&analysis, &job) {
            let ids: Vec<i64> = find
                .query_map(rusqlite::params![key, run], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            for dataset in ids {
                link.execute([run, dataset])?;
            }
        }
    }
    Ok(())
}

/// Schema version this engine writes.
pub const VERSION: i64 = MIGRATIONS.len() as i64;

/// Open (creating when `create`) the database at `path` and bring its
/// schema up to date.
pub fn open(path: &Path, create: bool) -> Result<Connection> {
    let flags = if create {
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
    } else {
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
    };
    let mut db = Connection::open_with_flags(path, flags).map_err(Error::database(path))?;
    // Several processes (server, CLI) may use one project: wait for locks
    // rather than fail, and let readers work during a write.
    db.busy_timeout(Duration::from_secs(30))
        .map_err(Error::database(path))?;
    db.pragma_update(None, "journal_mode", "WAL")
        .map_err(Error::database(path))?;
    db.pragma_update(None, "foreign_keys", "ON")
        .map_err(Error::database(path))?;
    migrate(&mut db, path)?;
    Ok(db)
}

fn migrate(db: &mut Connection, path: &Path) -> Result<()> {
    // Take the write lock before reading the version: two processes opening
    // a new or old project must not both apply a migration.
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(Error::database(path))?;
    let version: i64 = tx
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(Error::database(path))?;
    if version > VERSION {
        return Err(Error::invalid(format!(
            "project database {} has schema version {version}; this engine reads up to {VERSION}: \
             update accessmod",
            path.display()
        )));
    }
    for migration in MIGRATIONS.iter().skip(version as usize) {
        tx.execute_batch(migration.sql)
            .map_err(Error::database(path))?;
        if let Some(then) = migration.then {
            then(&tx).map_err(Error::database(path))?;
        }
    }
    if version < VERSION {
        tx.pragma_update(None, "user_version", VERSION)
            .map_err(Error::database(path))?;
    }
    tx.commit().map_err(Error::database(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_database_reaches_the_current_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.sqlite");
        let db = open(&path, true).unwrap();
        let version: i64 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, VERSION);
        drop(db);
        // Opening again runs no migration and keeps the version.
        let db = open(&path, false).unwrap();
        let version: i64 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, VERSION);
    }

    #[test]
    fn runs_of_a_version_1_project_get_their_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.sqlite");
        // A project database of version 1, with runs and datasets.
        let db = Connection::open(&path).unwrap();
        db.execute_batch(V1).unwrap();
        db.execute_batch(
            "INSERT INTO runs (id, analysis, tags, job, status, output_dir, started)
             VALUES (1, 'travel-time', '[]',
                     '{\"schema_version\": 1, \"dem\": \"./datasets/1.tif\",
                       \"land_cover\": \"datasets/9.tif\",
                       \"facilities\": {\"path\": \"datasets//2.fgb\", \"id_field\": \"datasets/3.tif\"},
                       \"scenario\": [{\"class\": 1, \"label\": \"datasets/3.tif\",
                                      \"mode\": \"motorized\", \"speed_kmh\": 10.0}],
                       \"method\": \"isotropic\", \"direction\": \"from_facilities\",
                       \"neighbourhood\": 8, \"output_dir\": \"runs/1\"}',
                     'done', 'runs/1', 0),
                    (2, 'travel-time', '[]', 'not json', 'failed', 'runs/2', 0);
             INSERT INTO datasets (id, class, tags, tags_key, file, state, origin, run, created)
             VALUES (1, 'rDem', '[]', 'a', 'datasets/1.tif', 'ready', '{}', NULL, 0),
                    (2, 'vFacility', '[]', 'b', 'datasets/2.fgb', 'ready', '{}', NULL, 0),
                    (3, 'rLandCoverMerged', '[]', 'c', 'datasets/3.tif', 'ready', '{}', NULL, 0),
                    (4, 'rTravelTime', '[]', 'd', 'runs/1', 'ready', '{}', 1, 0);",
        )
        .unwrap();
        db.pragma_update(None, "user_version", 1).unwrap();
        drop(db);
        let db = open(&path, false).unwrap();
        let mut stmt = db
            .prepare("SELECT run, dataset FROM run_inputs ORDER BY run, dataset")
            .unwrap();
        let links: Vec<(i64, i64)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        // The DEM (through `./`) and the facilities (through `//`); not a
        // string that only looks like a path (dataset 3), not its own output,
        // nothing from a job that no longer parses.
        assert_eq!(links, [(1, 1), (1, 2)]);
    }

    #[test]
    fn newer_database_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.sqlite");
        let db = open(&path, true).unwrap();
        db.pragma_update(None, "user_version", VERSION + 1).unwrap();
        drop(db);
        let err = open(&path, false).unwrap_err();
        assert!(err.to_string().contains("update accessmod"), "{err}");
    }

    #[test]
    fn missing_database_is_not_created_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.sqlite");
        assert!(open(&path, false).is_err());
        assert!(!path.exists());
    }
}
