//! SeaORM entity for the persisted `sessions` table.
//!
//! Mirrors the durable subset of [`crate::session::types::Session`]: identity
//! (id, token, user, home server), lifecycle state, and the timing fields
//! needed to enforce the suspend timeout across restarts. Live runtime handles
//! (surfaces, network bindings) are intentionally not persisted — a restarted
//! server cannot revive them (see [`crate::session::persistence`] for the
//! restore semantics).
//!
//! Timestamps are stored as Unix epoch seconds. The in-memory `Session` uses
//! monotonic [`std::time::Instant`]s, which are meaningless across process
//! restarts, so they are converted to/from wall-clock time at the persistence
//! boundary.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "sessions")]
pub struct Model {
    /// Session id ([`crate::session::SessionId::raw`]). Assigned by the
    /// registry, not the database, so auto-increment is disabled.
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: i64,
    /// The session token (locator credential). Not marked unique: in-memory
    /// uniqueness is enforced by the registry's token index, and a rebind race
    /// (old session not yet destroyed when a new one takes the token) must not
    /// make the persistence write fail.
    pub token: String,
    /// Lifecycle state as its display string ("creating", "active",
    /// "suspended", "destroyed").
    pub state: String,
    /// Authenticated username, if authentication completed.
    pub user: Option<String>,
    /// Cluster id of the server that owns this session.
    pub home_server: String,
    /// Session creation time, Unix epoch seconds.
    pub created_at: i64,
    /// When the session entered Suspended state, Unix epoch seconds.
    pub suspended_at: Option<i64>,
    /// Suspend timeout in seconds (suspended sessions past this are destroyed).
    pub suspend_timeout_secs: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
