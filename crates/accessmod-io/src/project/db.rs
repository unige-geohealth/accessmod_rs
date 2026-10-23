//! The project database: schema, migrations, connection settings.
//!
//! The schema version is SQLite's `user_version`; migrations run in order
//! when a project opens, each in its own transaction. A database newer
//! than this engine is refused rather than misread.

use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;

use crate::error::{Error, Result};

/// Migrations, in order; migration `i` brings the schema to version `i + 1`.
const MIGRATIONS: &[&str] = &[
    // 1: project, datasets, saved jobs, runs.
    "CREATE TABLE meta (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    CREATE TABLE datasets (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        class TEXT NOT NULL,
        tags TEXT NOT NULL,
        tags_key TEXT NOT NULL,
        file TEXT NOT NULL,
        state TEXT NOT NULL CHECK (state IN ('importing', 'ready')),
        origin TEXT NOT NULL,
        sha256 TEXT,
        bytes INTEGER,
        preview TEXT,
        created INTEGER NOT NULL
    );
    CREATE UNIQUE INDEX datasets_class_tags ON datasets (class, tags_key);
    CREATE TABLE jobs (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        analysis TEXT NOT NULL,
        name TEXT NOT NULL,
        document TEXT NOT NULL,
        updated INTEGER NOT NULL
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
    );",
];

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
    let version: i64 = db
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(Error::database(path))?;
    if version > VERSION {
        return Err(Error::invalid(format!(
            "project database {} has schema version {version}; this engine reads up to {VERSION}: \
             update accessmod",
            path.display()
        )));
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        let tx = db.transaction().map_err(Error::database(path))?;
        tx.execute_batch(sql).map_err(Error::database(path))?;
        tx.pragma_update(None, "user_version", i as i64 + 1)
            .map_err(Error::database(path))?;
        tx.commit().map_err(Error::database(path))?;
    }
    Ok(())
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
