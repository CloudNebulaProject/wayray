//! Optional session persistence (roadmap Phase 3.1).
//!
//! The in-memory [`super::SessionRegistry`] loses all sessions on server
//! restart. This module adds an optional SQLite-backed store (via SeaORM,
//! never raw SQL) so session *metadata* survives restarts — the groundwork
//! for state replication / HA.
//!
//! # Restore semantics
//!
//! A restarted server has no live Wayland surfaces, so persisted sessions
//! cannot be resumed with their old desktop content. What *is* durable — and
//! what SunRay-style hot-desking actually needs — is the token → session
//! identity binding. On startup with persistence enabled:
//!
//! - Sessions persisted as `Active` or `Suspended` are restored into the
//!   registry as `Suspended`, keeping their id, token, user, home server and
//!   suspend timeout. A returning client presenting its token resumes into a
//!   session with the same identity (and is counted/advertised to cluster
//!   peers as resumable), instead of being handed a brand-new anonymous
//!   session.
//! - The suspend clock restarts at server boot (`suspended_at = now`): the
//!   monotonic timestamps of the previous process are gone, and granting the
//!   full timeout again is the user-friendly choice after an outage.
//! - Sessions persisted as `Creating` never had a usable desktop and are
//!   dropped (and deleted from the store). `Destroyed` rows are stale
//!   leftovers and are likewise deleted.
//!
//! # Runtime / blocking tradeoff
//!
//! SeaORM is async, but the compositor hot path is a synchronous calloop
//! loop. [`SqliteStore`] therefore owns a small dedicated tokio runtime
//! (mirroring the peer-probe runtime in the headless backend) and applies
//! writes **fire-and-forget** through an ordered queue: `persist`/`remove`
//! return immediately and the write happens on the runtime's worker thread,
//! with failures logged. The calloop loop is never blocked by the database.
//! The tradeoff is a small window in which a crash can lose the most recent
//! transition(s); for suspend/resume metadata that is acceptable, and a
//! `flush` is available for tests and orderly shutdown. Startup restore
//! (`load_all`) is the one intentionally blocking call, before the event
//! loop starts.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveValue::Set, ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbErr,
    EntityTrait, Schema,
};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tracing::{error, warn};

use super::entity;
use super::types::{Session, SessionId, SessionState};

/// How long `flush` waits for the writer task before giving up. Only relevant
/// if the writer task died (e.g. panicked); normal flushes complete in
/// microseconds.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

/// A session as loaded back from a persistent store.
///
/// Plain data (no SeaORM types) so [`SessionStore`] implementations other
/// than SQLite remain possible (e.g. a replicated store for HA).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedSession {
    pub id: u64,
    pub token: String,
    pub state: SessionState,
    pub user: Option<String>,
    pub home_server: String,
    /// Creation time, Unix epoch seconds.
    pub created_at_epoch: i64,
    pub suspend_timeout: Duration,
}

/// Durable backing store for session metadata.
///
/// The registry calls `persist` on every state change (create, activate,
/// suspend, resume, user binding) and `remove` when a session is destroyed.
/// Implementations must apply operations in call order.
pub trait SessionStore: Send + Sync {
    /// Persist the session's current state (insert or update).
    fn persist(&self, session: &Session);

    /// Remove a session from the store (called on destroy).
    fn remove(&self, id: SessionId);

    /// Load all persisted sessions (startup restore).
    fn load_all(&self) -> Vec<PersistedSession>;

    /// Block until previously queued writes have been applied. No-op for
    /// synchronous stores.
    fn flush(&self) {}
}

/// The default no-op store: sessions live only in memory (current behavior
/// when `--state-db` is not given).
#[derive(Debug, Default)]
pub struct NullStore;

impl SessionStore for NullStore {
    fn persist(&self, _session: &Session) {}

    fn remove(&self, _id: SessionId) {}

    fn load_all(&self) -> Vec<PersistedSession> {
        Vec::new()
    }
}

/// Wall-clock snapshot of a session, taken synchronously at call time so the
/// async write observes the state as of the transition (not a later one).
#[derive(Debug)]
struct SessionRecord {
    id: i64,
    token: String,
    state: String,
    user: Option<String>,
    home_server: String,
    created_at: i64,
    suspended_at: Option<i64>,
    suspend_timeout_secs: i64,
}

/// Operations queued to the single writer task. A single ordered queue (not
/// bare `spawn`s) guarantees a `persist` followed by a `remove` for the same
/// session cannot be applied out of order.
enum Op {
    Save(SessionRecord),
    Delete(i64),
    Flush(std::sync::mpsc::Sender<()>),
}

/// SQLite-backed [`SessionStore`] using SeaORM.
pub struct SqliteStore {
    tx: UnboundedSender<Op>,
    db: DatabaseConnection,
    /// Dedicated runtime driving the writer task and startup queries.
    rt: tokio::runtime::Runtime,
}

impl SqliteStore {
    /// Open (creating if necessary) a SQLite database file.
    pub fn open_file(path: &Path) -> Result<Self, DbErr> {
        Self::open(&format!("sqlite://{}?mode=rwc", path.display()))
    }

    /// Open an in-memory database (tests).
    pub fn open_memory() -> Result<Self, DbErr> {
        Self::open("sqlite::memory:")
    }

    /// Connect to `url`, create the schema if missing, and start the writer.
    pub fn open(url: &str) -> Result<Self, DbErr> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("wrsrvd-state-db")
            .enable_all()
            .build()
            .map_err(|e| DbErr::Custom(format!("failed to build state-db runtime: {e}")))?;

        let db = rt.block_on(async {
            let mut opts = ConnectOptions::new(url.to_owned());
            // One connection: keeps writes serialized, and makes
            // `sqlite::memory:` sane (every new pooled connection would
            // otherwise open its own fresh, empty database).
            opts.max_connections(1).sqlx_logging(false);
            let db = Database::connect(opts).await?;

            // Create the schema programmatically from the entity — no
            // external migration CLI needed for a single-table store.
            let schema = Schema::new(db.get_database_backend());
            let mut create = schema.create_table_from_entity(entity::Entity);
            create.if_not_exists();
            db.execute(db.get_database_backend().build(&create)).await?;
            Ok::<_, DbErr>(db)
        })?;

        let (tx, mut rx) = unbounded_channel::<Op>();
        let writer_db = db.clone();
        rt.spawn(async move {
            while let Some(op) = rx.recv().await {
                match op {
                    Op::Save(record) => {
                        let id = record.id;
                        if let Err(e) = save_record(&writer_db, record).await {
                            error!(session_id = id, error = %e, "failed to persist session");
                        }
                    }
                    Op::Delete(id) => {
                        if let Err(e) = entity::Entity::delete_by_id(id).exec(&writer_db).await {
                            error!(session_id = id, error = %e, "failed to delete persisted session");
                        }
                    }
                    Op::Flush(done) => {
                        // All ops queued before this one have been applied.
                        let _ = done.send(());
                    }
                }
            }
        });

        Ok(Self { tx, db, rt })
    }
}

impl SessionStore for SqliteStore {
    fn persist(&self, session: &Session) {
        // Fire-and-forget: queue the snapshot; the writer task applies it.
        if self.tx.send(Op::Save(record_of(session))).is_err() {
            error!(id = %session.id, "session store writer is gone; state not persisted");
        }
    }

    fn remove(&self, id: SessionId) {
        if self.tx.send(Op::Delete(id.raw() as i64)).is_err() {
            error!(%id, "session store writer is gone; deletion not persisted");
        }
    }

    fn load_all(&self) -> Vec<PersistedSession> {
        // Drain queued writes first so the read sees a consistent snapshot.
        self.flush();
        let rows = match self.rt.block_on(entity::Entity::find().all(&self.db)) {
            Ok(rows) => rows,
            Err(e) => {
                error!(error = %e, "failed to load persisted sessions");
                return Vec::new();
            }
        };
        rows.into_iter()
            .filter_map(|row| {
                let Some(state) = parse_state(&row.state) else {
                    warn!(id = row.id, state = %row.state, "skipping persisted session with unknown state");
                    return None;
                };
                Some(PersistedSession {
                    id: row.id as u64,
                    token: row.token,
                    state,
                    user: row.user,
                    home_server: row.home_server,
                    created_at_epoch: row.created_at,
                    suspend_timeout: Duration::from_secs(row.suspend_timeout_secs.max(0) as u64),
                })
            })
            .collect()
    }

    fn flush(&self) {
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        if self.tx.send(Op::Flush(done_tx)).is_err() {
            return;
        }
        if done_rx.recv_timeout(FLUSH_TIMEOUT).is_err() {
            warn!("timed out waiting for session store flush");
        }
    }
}

impl Drop for SqliteStore {
    fn drop(&mut self) {
        // Best-effort: apply queued writes before the runtime is torn down so
        // an orderly shutdown does not lose the final transitions.
        self.flush();
    }
}

/// Open the SQLite store at `path`, returning it as a shareable trait object.
pub fn open_store(path: &Path) -> Result<Arc<dyn SessionStore>, DbErr> {
    Ok(Arc::new(SqliteStore::open_file(path)?))
}

/// Upsert one session row.
async fn save_record(db: &DatabaseConnection, record: SessionRecord) -> Result<(), DbErr> {
    let model = entity::ActiveModel {
        id: Set(record.id),
        token: Set(record.token),
        state: Set(record.state),
        user: Set(record.user),
        home_server: Set(record.home_server),
        created_at: Set(record.created_at),
        suspended_at: Set(record.suspended_at),
        suspend_timeout_secs: Set(record.suspend_timeout_secs),
    };
    entity::Entity::insert(model)
        .on_conflict(
            OnConflict::column(entity::Column::Id)
                .update_columns([
                    entity::Column::Token,
                    entity::Column::State,
                    entity::Column::User,
                    entity::Column::HomeServer,
                    entity::Column::CreatedAt,
                    entity::Column::SuspendedAt,
                    entity::Column::SuspendTimeoutSecs,
                ])
                .to_owned(),
        )
        .exec(db)
        .await?;
    Ok(())
}

/// Snapshot a live session into wall-clock persistable form. Monotonic
/// `Instant`s are converted to epoch seconds relative to now.
fn record_of(session: &Session) -> SessionRecord {
    let now_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    SessionRecord {
        id: session.id.raw() as i64,
        token: session.token.0.clone(),
        state: state_str(session.state).to_owned(),
        user: session.user.clone(),
        home_server: session.home_server.clone(),
        created_at: now_epoch - session.created_at.elapsed().as_secs() as i64,
        suspended_at: session
            .suspended_at
            .map(|t| now_epoch - t.elapsed().as_secs() as i64),
        suspend_timeout_secs: session.suspend_timeout.as_secs() as i64,
    }
}

fn state_str(state: SessionState) -> &'static str {
    match state {
        SessionState::Creating => "creating",
        SessionState::Active => "active",
        SessionState::Suspended => "suspended",
        SessionState::Destroyed => "destroyed",
    }
}

fn parse_state(s: &str) -> Option<SessionState> {
    match s {
        "creating" => Some(SessionState::Creating),
        "active" => Some(SessionState::Active),
        "suspended" => Some(SessionState::Suspended),
        "destroyed" => Some(SessionState::Destroyed),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::registry::SessionRegistry;
    use crate::session::types::SessionToken;

    fn memory_store() -> Arc<SqliteStore> {
        Arc::new(SqliteStore::open_memory().expect("in-memory sqlite store"))
    }

    #[test]
    fn save_load_roundtrip() {
        let store = memory_store();
        let mut session = Session::new(
            crate::session::SessionId::from_raw(7),
            SessionToken::new("roundtrip-tok"),
            "server-a",
        );
        session.user = Some("alice".to_string());
        session.transition(SessionState::Active).unwrap();

        store.persist(&session);

        let loaded = store.load_all();
        assert_eq!(loaded.len(), 1);
        let row = &loaded[0];
        assert_eq!(row.id, 7);
        assert_eq!(row.token, "roundtrip-tok");
        assert_eq!(row.state, SessionState::Active);
        assert_eq!(row.user.as_deref(), Some("alice"));
        assert_eq!(row.home_server, "server-a");
        assert_eq!(row.suspend_timeout, Duration::from_secs(24 * 60 * 60));
    }

    #[test]
    fn state_transitions_are_persisted() {
        let store = memory_store();
        let mut reg = SessionRegistry::with_cluster("server-a", 50)
            .with_store(store.clone() as Arc<dyn SessionStore>);

        let id = reg.create_session(SessionToken::new("tok"));
        assert_eq!(store.load_all()[0].state, SessionState::Creating);

        reg.activate(id).unwrap();
        assert_eq!(store.load_all()[0].state, SessionState::Active);

        reg.suspend(id).unwrap();
        assert_eq!(store.load_all()[0].state, SessionState::Suspended);

        // Resume is a persisted transition too.
        reg.activate(id).unwrap();
        assert_eq!(store.load_all()[0].state, SessionState::Active);

        reg.set_user(id, "bob".to_string());
        assert_eq!(store.load_all()[0].user.as_deref(), Some("bob"));
    }

    #[test]
    fn destroy_deletes_from_store() {
        let store = memory_store();
        let mut reg = SessionRegistry::new().with_store(store.clone() as Arc<dyn SessionStore>);

        let id = reg.create_session(SessionToken::new("tok"));
        reg.activate(id).unwrap();
        assert_eq!(store.load_all().len(), 1);

        reg.destroy(id).unwrap();
        assert!(store.load_all().is_empty());
    }

    #[test]
    fn expired_cleanup_deletes_from_store() {
        let store = memory_store();
        let mut reg = SessionRegistry::new().with_store(store.clone() as Arc<dyn SessionStore>);

        let id = reg.create_session(SessionToken::new("tok"));
        reg.activate(id).unwrap();
        reg.suspend(id).unwrap();
        reg.set_suspend_timeout(id, Duration::ZERO);

        let expired = reg.cleanup_expired();
        assert_eq!(expired, vec![id]);
        assert!(store.load_all().is_empty());
    }

    #[test]
    fn restart_restores_sessions_as_suspended() {
        let store = memory_store();

        // "First boot": one active, one suspended, one stuck in Creating.
        let active_tok = SessionToken::new("active-tok");
        let suspended_tok = SessionToken::new("suspended-tok");
        let creating_tok = SessionToken::new("creating-tok");
        {
            let mut reg = SessionRegistry::with_cluster("server-a", 50)
                .with_store(store.clone() as Arc<dyn SessionStore>);
            let a = reg.create_session(active_tok.clone());
            reg.activate(a).unwrap();
            reg.set_user(a, "alice".to_string());

            let s = reg.create_session(suspended_tok.clone());
            reg.activate(s).unwrap();
            reg.suspend(s).unwrap();

            let _c = reg.create_session(creating_tok.clone());
        }

        // "Restart": a fresh registry restores identity bindings from the
        // same store.
        let mut reg = SessionRegistry::with_cluster("server-a", 50)
            .with_store(store.clone() as Arc<dyn SessionStore>);
        let restored = reg.restore_persisted();
        assert_eq!(restored, 2);

        // Active and Suspended both come back as Suspended (no live
        // surfaces after a restart), resumable by token.
        for tok in [&active_tok, &suspended_tok] {
            let session = reg.find_by_token(tok).expect("restored session");
            assert_eq!(session.state, SessionState::Suspended);
            assert!(reg.is_resumable(tok).is_some());
        }
        let alice = reg.find_by_token(&active_tok).unwrap();
        assert_eq!(alice.user.as_deref(), Some("alice"));
        assert_eq!(alice.home_server, "server-a");

        // A half-created session never had a desktop: dropped and purged
        // from the store.
        assert!(reg.find_by_token(&creating_tok).is_none());
        let rows = store.load_all();
        assert_eq!(rows.len(), 2);
        // The store now reflects the demotion to Suspended.
        assert!(rows.iter().all(|r| r.state == SessionState::Suspended));

        // Session ids keep counting past the restored ones — ids 1..3 were
        // used before the restart, so the next id is 4.
        let new_id = reg.create_session(SessionToken::new("new-tok"));
        assert_eq!(new_id.raw(), 4);

        // A restored session participates in the normal lifecycle: resume it.
        let id = reg.is_resumable(&suspended_tok).unwrap();
        reg.activate(id).unwrap();
        assert_eq!(reg.get(id).unwrap().state, SessionState::Active);
    }

    #[test]
    fn restore_with_null_store_is_empty() {
        let mut reg = SessionRegistry::new();
        assert_eq!(reg.restore_persisted(), 0);
        assert_eq!(reg.list().count(), 0);
    }
}
