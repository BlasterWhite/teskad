//! The database: one SQLite file, one connection, versioned migrations run at
//! start (docs/native/cloud.md, "What the server stores").
//!
//! One connection behind a mutex, used from blocking threads: SQLite takes
//! one writer at a time whatever is in front of it, every statement here is
//! short, and one connection is what makes "a transaction" mean the same
//! thing everywhere. A request never holds the lock across an `await`: it
//! hands a closure to [`Db::call`], which runs it whole on a blocking thread.
//!
//! Times are milliseconds since the Unix epoch. Hashes are 32-byte BLOBs.

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rusqlite::Connection;

use crate::refusal::Refusal;

/// Each entry takes the schema from the version of its index to the next;
/// `PRAGMA user_version` says how many a file has had. An entry is never
/// edited once it has shipped: a change is a new entry.
const MIGRATIONS: &[&str] = &[
    // 1: the account, its devices, their statistics, and sign-ins in progress.
    "
    CREATE TABLE accounts (
        id          TEXT PRIMARY KEY,
        created_at  INTEGER NOT NULL
    ) STRICT;

    -- What a provider said about the person, at their last sign-in with it.
    -- An identity belongs to one account. `email` is only ever an address the
    -- provider says it verified. The account shows the profile of its first
    -- identity.
    CREATE TABLE identities (
        provider          TEXT NOT NULL,
        provider_user_id  TEXT NOT NULL,
        account_id        TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        email             TEXT,
        display_name      TEXT NOT NULL,
        avatar_url        TEXT,
        created_at        INTEGER NOT NULL,
        updated_at        INTEGER NOT NULL,
        PRIMARY KEY (provider, provider_user_id)
    ) STRICT;
    CREATE INDEX identities_account ON identities(account_id);
    CREATE INDEX identities_email ON identities(email) WHERE email IS NOT NULL;

    -- One per machine and account. `token_hash` is the SHA-256 of the device
    -- token, and NULL once the device is signed out: the token is destroyed,
    -- not flagged.
    CREATE TABLE devices (
        id            TEXT PRIMARY KEY,
        account_id    TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        machine_id    TEXT NOT NULL,
        name          TEXT NOT NULL,
        os            TEXT NOT NULL,
        created_at    INTEGER NOT NULL,
        last_seen_at  INTEGER NOT NULL,
        token_hash    BLOB UNIQUE,
        revoked_at    INTEGER,
        UNIQUE (account_id, machine_id),
        CHECK ((token_hash IS NULL) = (revoked_at IS NOT NULL))
    ) STRICT;

    CREATE TABLE stats (
        device_id           TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
        day                 TEXT NOT NULL,
        model               TEXT NOT NULL,
        project             TEXT NOT NULL,
        input_tokens        INTEGER NOT NULL CHECK (input_tokens >= 0),
        output_tokens       INTEGER NOT NULL CHECK (output_tokens >= 0),
        cache_read_tokens   INTEGER NOT NULL CHECK (cache_read_tokens >= 0),
        cache_write_tokens  INTEGER NOT NULL CHECK (cache_write_tokens >= 0),
        cost_usd_micros     INTEGER NOT NULL CHECK (cost_usd_micros >= 0),
        active_seconds      INTEGER NOT NULL CHECK (active_seconds >= 0),
        sessions            INTEGER NOT NULL CHECK (sessions >= 0),
        updated_at          INTEGER NOT NULL,
        PRIMARY KEY (device_id, day, model, project)
    ) STRICT, WITHOUT ROWID;

    -- The one-time code a finished sign-in hands the desktop through the
    -- browser. `code_hash` is its SHA-256; `challenge` is the desktop's PKCE
    -- challenge, which the exchange checks its verifier against.
    CREATE TABLE link_codes (
        code_hash   BLOB PRIMARY KEY,
        account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        challenge   TEXT NOT NULL,
        expires_at  INTEGER NOT NULL,
        used_at     INTEGER
    ) STRICT;
    CREATE INDEX link_codes_expiry ON link_codes(expires_at);

    -- A sign-in between /v1/auth/start and the provider's callback: what the
    -- desktop sent (its port, state, challenge and machine name) and what the
    -- server drew for the provider (its own state, kept as its SHA-256, and
    -- its own PKCE verifier). Filled by the sign-in routes.
    CREATE TABLE auth_states (
        state_hash         BLOB PRIMARY KEY,
        provider           TEXT NOT NULL,
        port               INTEGER NOT NULL,
        desktop_state      TEXT NOT NULL,
        challenge          TEXT NOT NULL,
        machine            TEXT NOT NULL,
        provider_verifier  TEXT NOT NULL,
        created_at         INTEGER NOT NULL,
        expires_at         INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX auth_states_expiry ON auth_states(expires_at);
    ",
];

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    /// Opens the file (created when it is not there, in a folder that is) and
    /// brings its schema up to date.
    pub fn open(path: &Path) -> Result<Self, String> {
        let conn = Connection::open(path)
            .map_err(|err| format!("could not open the database {}: {err}", path.display()))?;
        Self::prepare(conn)
            .map_err(|err| format!("could not prepare the database {}: {err}", path.display()))
    }

    /// A database that lives as long as this value, for the tests.
    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self::prepare(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn prepare(mut conn: Connection) -> Result<Self, String> {
        let sql = |err: rusqlite::Error| err.to_string();
        conn.busy_timeout(Duration::from_secs(5)).map_err(sql)?;
        // Readers (a backup's copy, the sqlite3 shell) do not block the
        // server and are not blocked by it. `journal_mode` answers with the
        // mode it settled on, which a database in memory keeps as `memory`.
        conn.query_row("PRAGMA journal_mode = WAL", [], |row| {
            row.get::<_, String>(0)
        })
        .map_err(sql)?;
        // Off by default in SQLite, and per connection: without it no cascade
        // runs and deleting an account would leave its rows behind.
        // What is deleted is overwritten in the file, not only unlinked.
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA secure_delete = ON;")
            .map_err(sql)?;
        migrate(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Runs `work` with the connection, on a blocking thread, and gives back
    /// what it returns. `work` is the unit of exclusion: nothing else touches
    /// the database while it runs.
    pub async fn call<T, F>(&self, work: F) -> Result<T, Refusal>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, Refusal> + Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            // A panic while the lock was held rolled its transaction back
            // when it unwound: the connection is as good as before.
            let mut conn = conn.lock().unwrap_or_else(PoisonError::into_inner);
            work(&mut conn)
        })
        .await
        .unwrap_or_else(|_| Err(Refusal::internal("a database task panicked")))
    }
}

fn migrate(conn: &mut Connection) -> Result<(), String> {
    let sql = |err: rusqlite::Error| err.to_string();
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(sql)?;
    let done = usize::try_from(version).map_err(|_| format!("its version is {version}"))?;
    if done > MIGRATIONS.len() {
        // Written by a newer server: this one does not know what is in it.
        return Err(format!(
            "its schema is version {done} and this server knows up to {}",
            MIGRATIONS.len()
        ));
    }
    for (index, migration) in MIGRATIONS.iter().enumerate().skip(done) {
        // The schema and its version move together or not at all.
        let tx = conn.transaction().map_err(sql)?;
        tx.execute_batch(migration)
            .map_err(|err| format!("migration {}: {err}", index + 1))?;
        tx.pragma_update(None, "user_version", index as i64 + 1)
            .map_err(sql)?;
        tx.commit().map_err(sql)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pragma<T: rusqlite::types::FromSql>(conn: &Connection, name: &str) -> T {
        conn.query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
            .unwrap()
    }

    #[tokio::test]
    async fn a_new_file_gets_the_schema_wal_and_foreign_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cloud.sqlite");
        let db = Db::open(&path).unwrap();
        db.call(|conn| {
            assert_eq!(pragma::<i64>(conn, "user_version"), MIGRATIONS.len() as i64);
            assert_eq!(pragma::<String>(conn, "journal_mode"), "wal");
            assert_eq!(pragma::<i64>(conn, "foreign_keys"), 1);
            let tables: Vec<String> = conn
                .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")?
                .query_map([], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            assert_eq!(
                tables,
                [
                    "accounts",
                    "auth_states",
                    "devices",
                    "identities",
                    "link_codes",
                    "stats"
                ]
            );
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn opening_again_keeps_what_is_there() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cloud.sqlite");
        let db = Db::open(&path).unwrap();
        db.call(|conn| {
            conn.execute("INSERT INTO accounts (id, created_at) VALUES ('a', 1)", [])?;
            Ok(())
        })
        .await
        .unwrap();
        drop(db);

        let db = Db::open(&path).unwrap();
        let accounts: i64 = db
            .call(|conn| Ok(conn.query_row("SELECT count(*) FROM accounts", [], |r| r.get(0))?))
            .await
            .unwrap();
        assert_eq!(accounts, 1);
    }

    #[test]
    fn a_file_from_a_newer_server_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cloud.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", MIGRATIONS.len() as i64 + 1)
            .unwrap();
        drop(conn);
        let err = Db::open(&path).err().unwrap();
        assert!(err.contains("version"), "{err}");
    }

    #[test]
    fn a_folder_that_does_not_exist_is_said() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("cloud.sqlite");
        let err = Db::open(&path).err().unwrap();
        assert!(err.contains("could not open the database"), "{err}");
    }

    #[tokio::test]
    async fn a_row_cannot_name_an_account_that_does_not_exist() {
        let db = Db::in_memory();
        db.call(|conn| {
            let orphan = conn.execute(
                "INSERT INTO devices (id, account_id, machine_id, name, os, created_at, \
                 last_seen_at, token_hash) VALUES ('d', 'nobody', 'm', 'n', 'linux', 1, 1, x'00')",
                [],
            );
            assert!(orphan.is_err());
            Ok(())
        })
        .await
        .unwrap();
    }
}
