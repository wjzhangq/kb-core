use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};
use std::sync::Once;

use crate::lock::{KBLock, KBLockError};
// migrations is declared as a submodule below; no separate use needed.

pub mod migrations;
pub mod schema;

static VEC_EXT_INIT: Once = Once::new();

fn ensure_vec_extension() {
    VEC_EXT_INIT.call_once(|| {
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
                *const (),
                unsafe extern "C" fn(
                    *mut rusqlite::ffi::sqlite3,
                    *mut *const std::os::raw::c_char,
                    *const rusqlite::ffi::sqlite3_api_routines,
                ) -> std::os::raw::c_int,
            >(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
    });
}

pub struct DbConn {
    pub conn: Connection,
    pub data_dir: PathBuf,
    pub _lock: Option<KBLock>,
}

impl DbConn {
    /// Open in read-write mode; acquires flock.
    pub fn open_writer(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)
            .with_context(|| format!("create dataDir {:?}", data_dir))?;

        let lock = KBLock::acquire(data_dir)
            .map_err(|e| match e {
                KBLockError::Locked { held_by } => anyhow::anyhow!(
                    "KBLockError: another writer holds the lock (held_by: {:?})", held_by
                ),
                KBLockError::Io(e) => anyhow::anyhow!("KBLockError IO: {}", e),
            })?;

        ensure_vec_extension();

        let db_path = data_dir.join("kb.db");
        let conn = Connection::open(&db_path)
            .with_context(|| format!("open SQLite {:?}", db_path))?;

        Self::configure(&conn)?;
        migrations::run_migrations(&conn).context("run migrations")?;

        Ok(DbConn { conn, data_dir: data_dir.to_path_buf(), _lock: Some(lock) })
    }

    /// Open in read-only mode; no flock acquired.
    pub fn open_readonly(data_dir: &Path) -> Result<Self> {
        ensure_vec_extension();

        let db_path = data_dir.join("kb.db");
        let conn = Connection::open_with_flags(
            &db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| format!("open SQLite read-only {:?}", db_path))?;

        Ok(DbConn { conn, data_dir: data_dir.to_path_buf(), _lock: None })
    }

    fn configure(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             PRAGMA synchronous=NORMAL;
             PRAGMA cache_size=-32000;",
        )
        .context("configure SQLite pragmas")?;
        Ok(())
    }

    /// Release all OS handles this connection holds — the kb.db/-wal/-shm file
    /// descriptors and the `.writer.lock` advisory lock — without consuming the
    /// `DbConn` (which lives behind `Arc<Mutex<..>>` and can't be moved out on
    /// close). The live file-backed `Connection` is swapped for a throwaway
    /// in-memory one and dropped, which closes the SQLite handles and
    /// checkpoints/removes the WAL sidecar files; dropping `_lock` releases the
    /// flock. Idempotent: a second call just replaces an already-in-memory conn.
    ///
    /// Required on Windows, where an open file handle blocks `unlink`. On POSIX
    /// unlink succeeds regardless, so this only affects whether a caller can
    /// delete the data dir immediately after `close()`.
    pub fn release_handles(&mut self) {
        // In-memory open needs no filesystem access and effectively never fails;
        // if it somehow did, keep the existing connection rather than panic.
        if let Ok(mem) = Connection::open_in_memory() {
            let old = std::mem::replace(&mut self.conn, mem);
            drop(old);
        }
        self._lock = None;
    }
}
