use std::collections::HashMap;
use std::time::{Duration, Instant};

use tracing::{info, warn};

use super::types::{Session, SessionId, SessionState, SessionToken, SessionTransitionError};
use wayray_protocol::cluster::DEFAULT_CAPACITY;

/// How long a cached "this token lives on server X" affinity entry is trusted
/// before it must be re-confirmed by a fresh peer probe. Without expiry a stale
/// entry (the session moved or died) would redirect a reconnecting client to a
/// server that no longer hosts it — looping until the redirect limit drops the
/// session.
const REMOTE_SESSION_TTL: Duration = Duration::from_secs(30);

/// A cached remote-session affinity record: which server hosts the token, and
/// when we learned it (for TTL expiry).
#[derive(Debug, Clone)]
struct RemoteEntry {
    server_id: String,
    noted_at: Instant,
}

/// Where a session bound to a token currently lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionLocation {
    /// The session lives on this server.
    Local(SessionId),
    /// The session lives on another server (by id).
    Remote(String),
    /// No session is known for the token anywhere.
    Unknown,
}

/// In-memory session registry.
///
/// Provides O(1) lookup by both session ID and token. Tracks all sessions
/// including suspended ones (until they time out and are cleaned up).
///
/// For multi-server deployments the registry also knows the local server's id
/// (stamped onto every session it creates) and a best-effort map of tokens
/// whose sessions are known to live on other servers, populated from cluster
/// configuration or peer queries.
pub struct SessionRegistry {
    sessions: HashMap<SessionId, Session>,
    /// Reverse index: token → session ID for fast lookup on client connect.
    token_index: HashMap<SessionToken, SessionId>,
    next_id: u64,
    /// This server's id within the cluster (empty for single-server mode).
    local_server_id: String,
    /// Tokens known to live on a remote server, with the time the affinity was
    /// learned (entries past [`REMOTE_SESSION_TTL`] are treated as unknown).
    remote_sessions: HashMap<SessionToken, RemoteEntry>,
    /// Maximum number of sessions this server will host (for load factor).
    capacity: u32,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            token_index: HashMap::new(),
            next_id: 1,
            local_server_id: String::new(),
            remote_sessions: HashMap::new(),
            capacity: DEFAULT_CAPACITY,
        }
    }

    /// Create a registry with an explicit local server id and capacity.
    pub fn with_cluster(local_server_id: impl Into<String>, capacity: u32) -> Self {
        Self {
            local_server_id: local_server_id.into(),
            capacity: capacity.max(1),
            ..Self::new()
        }
    }

    /// This server's id (empty for single-server deployments).
    pub fn local_server_id(&self) -> &str {
        &self.local_server_id
    }

    /// Configured session capacity for this server.
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Mark a token as living on a remote server (from cluster config or a
    /// peer query). Used so a reconnecting client can be redirected home. The
    /// entry is timestamped and expires after [`REMOTE_SESSION_TTL`].
    pub fn note_remote_session(&mut self, token: SessionToken, server_id: impl Into<String>) {
        self.remote_sessions.insert(
            token,
            RemoteEntry {
                server_id: server_id.into(),
                noted_at: Instant::now(),
            },
        );
    }

    /// Determine where the session bound to `token` lives. A remote affinity
    /// entry older than [`REMOTE_SESSION_TTL`] is treated as `Unknown` so the
    /// caller re-probes peers instead of redirecting on stale information.
    pub fn locate(&self, token: &SessionToken) -> SessionLocation {
        if let Some(id) = self.token_index.get(token) {
            return SessionLocation::Local(*id);
        }
        if let Some(entry) = self.remote_sessions.get(token)
            && entry.noted_at.elapsed() < REMOTE_SESSION_TTL
        {
            return SessionLocation::Remote(entry.server_id.clone());
        }
        SessionLocation::Unknown
    }

    /// Drop remote-affinity entries that have passed [`REMOTE_SESSION_TTL`].
    /// Called periodically alongside session cleanup.
    pub fn prune_remote_sessions(&mut self) {
        self.remote_sessions
            .retain(|_, e| e.noted_at.elapsed() < REMOTE_SESSION_TTL);
    }

    /// Current load factor: active sessions / capacity, clamped to [0, 1+].
    pub fn load_factor(&self) -> f32 {
        let active = self.count_by_state(SessionState::Active) as f32;
        active / self.capacity.max(1) as f32
    }

    /// Create a new session for the given token. Returns the session ID.
    /// The session is stamped with this server's id as its `home_server`.
    pub fn create_session(&mut self, token: SessionToken) -> SessionId {
        let id = SessionId::from_raw(self.next_id);
        self.next_id += 1;

        let session = Session::new(id, token.clone(), self.local_server_id.clone());
        // A token we now host locally is no longer "remote".
        self.remote_sessions.remove(&token);
        self.token_index.insert(token.clone(), id);
        self.sessions.insert(id, session);

        // The token is a session credential; log only the id and home server.
        info!(%id, home_server = %self.local_server_id, "session created");
        id
    }

    /// Look up a session by its token.
    pub fn find_by_token(&self, token: &SessionToken) -> Option<&Session> {
        let id = self.token_index.get(token)?;
        self.sessions.get(id)
    }

    /// Look up a session by ID.
    pub fn get(&self, id: SessionId) -> Option<&Session> {
        self.sessions.get(&id)
    }

    /// Determine whether a session bound to `token` can be resumed by a
    /// reconnecting client (hot-desking).
    ///
    /// Returns `Some(id)` when a session exists in `Suspended` state (the
    /// normal disconnect-then-reconnect path) or in `Active` state. An
    /// `Active` session means the previous endpoint vanished without a clean
    /// disconnect; the new client may rebind ("steal") it. Returns `None`
    /// for `Creating`/`Destroyed` sessions or when no session is bound.
    pub fn is_resumable(&self, token: &SessionToken) -> Option<SessionId> {
        let session = self.find_by_token(token)?;
        match session.state {
            SessionState::Suspended | SessionState::Active => Some(session.id),
            SessionState::Creating | SessionState::Destroyed => None,
        }
    }

    /// Transition a session to a new state.
    pub fn transition(
        &mut self,
        id: SessionId,
        next: SessionState,
    ) -> Result<(), SessionTransitionError> {
        let session = self.sessions.get_mut(&id).ok_or(SessionTransitionError {
            session_id: id,
            from: SessionState::Destroyed,
            to: next,
        })?;

        let from = session.state;
        session.transition(next)?;
        info!(%id, %from, %next, "session state transition");

        // Clean up destroyed sessions from the index.
        if next == SessionState::Destroyed {
            self.token_index.remove(&session.token);
        }

        Ok(())
    }

    /// Activate a session (Creating → Active or Suspended → Active).
    pub fn activate(&mut self, id: SessionId) -> Result<(), SessionTransitionError> {
        self.transition(id, SessionState::Active)
    }

    /// Suspend a session (Active → Suspended).
    pub fn suspend(&mut self, id: SessionId) -> Result<(), SessionTransitionError> {
        self.transition(id, SessionState::Suspended)
    }

    /// Destroy a session.
    pub fn destroy(&mut self, id: SessionId) -> Result<(), SessionTransitionError> {
        self.transition(id, SessionState::Destroyed)
    }

    /// Set the user for a session (after authentication).
    pub fn set_user(&mut self, id: SessionId, user: String) {
        if let Some(session) = self.sessions.get_mut(&id) {
            session.user = Some(user);
        }
    }

    /// Clean up expired suspended sessions. Returns IDs of destroyed sessions.
    pub fn cleanup_expired(&mut self) -> Vec<SessionId> {
        let expired: Vec<SessionId> = self
            .sessions
            .values()
            .filter(|s| s.is_suspend_expired())
            .map(|s| s.id)
            .collect();

        for id in &expired {
            if let Err(e) = self.destroy(*id) {
                warn!(%id, error = %e, "failed to destroy expired session");
            } else {
                info!(%id, "expired suspended session destroyed");
            }
        }

        expired
    }

    /// Remove destroyed sessions from memory entirely.
    pub fn purge_destroyed(&mut self) {
        self.sessions
            .retain(|_, s| s.state != SessionState::Destroyed);
    }

    /// List all sessions (for admin queries).
    pub fn list(&self) -> impl Iterator<Item = &Session> {
        self.sessions.values()
    }

    /// Tokens of sessions this server hosts in a resumable state (`Active` or
    /// `Suspended`). Published to the network thread so peers can discover that
    /// a token's session lives here via a `SessionLookupRequest` probe.
    pub fn resumable_tokens(&self) -> Vec<String> {
        self.sessions
            .values()
            .filter(|s| matches!(s.state, SessionState::Active | SessionState::Suspended))
            .map(|s| s.token.0.clone())
            .collect()
    }

    /// Count sessions by state.
    pub fn count_by_state(&self, state: SessionState) -> usize {
        self.sessions.values().filter(|s| s.state == state).count()
    }
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_lookup() {
        let mut reg = SessionRegistry::new();
        let token = SessionToken::new("abc-123");
        let id = reg.create_session(token.clone());

        assert!(reg.get(id).is_some());
        assert!(reg.find_by_token(&token).is_some());
        assert_eq!(reg.find_by_token(&token).unwrap().id, id);
    }

    #[test]
    fn lifecycle_through_registry() {
        let mut reg = SessionRegistry::new();
        let token = SessionToken::new("tok");
        let id = reg.create_session(token.clone());

        assert_eq!(reg.get(id).unwrap().state, SessionState::Creating);

        reg.activate(id).unwrap();
        assert_eq!(reg.get(id).unwrap().state, SessionState::Active);

        reg.suspend(id).unwrap();
        assert_eq!(reg.get(id).unwrap().state, SessionState::Suspended);

        // Resume
        reg.activate(id).unwrap();
        assert_eq!(reg.get(id).unwrap().state, SessionState::Active);

        reg.destroy(id).unwrap();
        // Token index should be cleaned up
        assert!(reg.find_by_token(&token).is_none());
    }

    #[test]
    fn cleanup_expired() {
        let mut reg = SessionRegistry::new();
        let id = reg.create_session(SessionToken::new("t1"));
        reg.activate(id).unwrap();
        reg.suspend(id).unwrap();

        // Set zero timeout so it expires immediately
        reg.sessions.get_mut(&id).unwrap().suspend_timeout = std::time::Duration::ZERO;

        let expired = reg.cleanup_expired();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0], id);
        assert_eq!(reg.get(id).unwrap().state, SessionState::Destroyed);
    }

    #[test]
    fn purge_destroyed() {
        let mut reg = SessionRegistry::new();
        let id = reg.create_session(SessionToken::new("t"));
        reg.activate(id).unwrap();
        reg.destroy(id).unwrap();

        assert!(reg.get(id).is_some()); // Still in memory
        reg.purge_destroyed();
        assert!(reg.get(id).is_none()); // Gone
    }

    #[test]
    fn multiple_sessions() {
        let mut reg = SessionRegistry::new();
        let id1 = reg.create_session(SessionToken::new("tok-a"));
        let id2 = reg.create_session(SessionToken::new("tok-b"));

        assert_ne!(id1, id2);
        reg.activate(id1).unwrap();
        reg.activate(id2).unwrap();

        assert_eq!(reg.count_by_state(SessionState::Active), 2);

        reg.suspend(id1).unwrap();
        assert_eq!(reg.count_by_state(SessionState::Active), 1);
        assert_eq!(reg.count_by_state(SessionState::Suspended), 1);
    }

    #[test]
    fn is_resumable_across_lifecycle() {
        let mut reg = SessionRegistry::new();
        let token = SessionToken::new("resume-tok");
        let id = reg.create_session(token.clone());

        // Creating → not resumable yet.
        assert_eq!(reg.is_resumable(&token), None);

        // Active → resumable (steal path).
        reg.activate(id).unwrap();
        assert_eq!(reg.is_resumable(&token), Some(id));

        // Suspended → resumable (normal reconnect path).
        reg.suspend(id).unwrap();
        assert_eq!(reg.is_resumable(&token), Some(id));

        // Destroyed → not resumable, token index cleared.
        reg.activate(id).unwrap();
        reg.destroy(id).unwrap();
        assert_eq!(reg.is_resumable(&token), None);

        // Unknown token → not resumable.
        assert_eq!(reg.is_resumable(&SessionToken::new("nope")), None);
    }

    #[test]
    fn create_session_stamps_home_server() {
        let mut reg = SessionRegistry::with_cluster("server-a", 50);
        let id = reg.create_session(SessionToken::new("tok"));
        assert_eq!(reg.get(id).unwrap().home_server, "server-a");
    }

    #[test]
    fn locate_local_remote_unknown() {
        let mut reg = SessionRegistry::with_cluster("server-a", 50);
        let local_token = SessionToken::new("local-tok");
        let id = reg.create_session(local_token.clone());

        // Local session.
        assert_eq!(reg.locate(&local_token), SessionLocation::Local(id));

        // Remote session noted from cluster knowledge.
        let remote_token = SessionToken::new("remote-tok");
        reg.note_remote_session(remote_token.clone(), "server-b");
        assert_eq!(
            reg.locate(&remote_token),
            SessionLocation::Remote("server-b".to_string())
        );

        // Unknown token.
        assert_eq!(
            reg.locate(&SessionToken::new("nope")),
            SessionLocation::Unknown
        );
    }

    #[test]
    fn remote_affinity_expires_and_prunes() {
        let mut reg = SessionRegistry::with_cluster("server-a", 50);
        let token = SessionToken::new("remote-tok");
        reg.note_remote_session(token.clone(), "server-b");
        assert_eq!(
            reg.locate(&token),
            SessionLocation::Remote("server-b".to_string())
        );

        // Age the entry past the TTL (skip if the monotonic clock can't go back
        // far enough, e.g. very early in process life).
        if let Some(old) = Instant::now().checked_sub(REMOTE_SESSION_TTL + Duration::from_secs(1)) {
            reg.remote_sessions.get_mut(&token).unwrap().noted_at = old;
            // Stale affinity is reported as Unknown so the caller re-probes.
            assert_eq!(reg.locate(&token), SessionLocation::Unknown);
            // Pruning removes the dead entry.
            reg.prune_remote_sessions();
            assert!(reg.remote_sessions.is_empty());
        }
    }

    #[test]
    fn creating_local_clears_remote_marker() {
        let mut reg = SessionRegistry::with_cluster("server-a", 50);
        let token = SessionToken::new("tok");
        reg.note_remote_session(token.clone(), "server-b");
        let id = reg.create_session(token.clone());
        // Now hosted locally; remote marker must be gone.
        assert_eq!(reg.locate(&token), SessionLocation::Local(id));
    }

    #[test]
    fn load_factor_computes_active_over_capacity() {
        let mut reg = SessionRegistry::with_cluster("server-a", 4);
        assert_eq!(reg.load_factor(), 0.0);

        let id1 = reg.create_session(SessionToken::new("a"));
        let id2 = reg.create_session(SessionToken::new("b"));
        reg.activate(id1).unwrap();
        reg.activate(id2).unwrap();
        // 2 active / capacity 4 = 0.5
        assert!((reg.load_factor() - 0.5).abs() < f32::EPSILON);
        assert_eq!(reg.capacity(), 4);
        assert_eq!(reg.local_server_id(), "server-a");
    }

    #[test]
    fn resumable_tokens_lists_active_and_suspended() {
        let mut reg = SessionRegistry::with_cluster("server-a", 50);
        let active = reg.create_session(SessionToken::new("active-tok"));
        let suspended = reg.create_session(SessionToken::new("suspended-tok"));
        let creating = reg.create_session(SessionToken::new("creating-tok"));

        reg.activate(active).unwrap();
        reg.activate(suspended).unwrap();
        reg.suspend(suspended).unwrap();
        // `creating` stays in Creating; it is not yet resumable.
        let _ = creating;

        let mut tokens = reg.resumable_tokens();
        tokens.sort();
        assert_eq!(tokens, vec!["active-tok", "suspended-tok"]);

        // A destroyed session drops out of the published set.
        reg.destroy(active).unwrap();
        let tokens = reg.resumable_tokens();
        assert_eq!(tokens, vec!["suspended-tok".to_string()]);
    }

    #[test]
    fn set_user() {
        let mut reg = SessionRegistry::new();
        let id = reg.create_session(SessionToken::new("t"));
        assert!(reg.get(id).unwrap().user.is_none());

        reg.set_user(id, "alice".to_string());
        assert_eq!(reg.get(id).unwrap().user.as_deref(), Some("alice"));
    }
}
