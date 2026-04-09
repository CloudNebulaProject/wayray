use std::collections::HashMap;

use tracing::{info, warn};

use super::types::{Session, SessionId, SessionState, SessionToken, SessionTransitionError};

/// In-memory session registry.
///
/// Provides O(1) lookup by both session ID and token. Tracks all sessions
/// including suspended ones (until they time out and are cleaned up).
pub struct SessionRegistry {
    sessions: HashMap<SessionId, Session>,
    /// Reverse index: token → session ID for fast lookup on client connect.
    token_index: HashMap<SessionToken, SessionId>,
    next_id: u64,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            token_index: HashMap::new(),
            next_id: 1,
        }
    }

    /// Create a new session for the given token. Returns the session ID.
    pub fn create_session(&mut self, token: SessionToken) -> SessionId {
        let id = SessionId::from_raw(self.next_id);
        self.next_id += 1;

        let session = Session::new(id, token.clone());
        self.token_index.insert(token.clone(), id);
        self.sessions.insert(id, session);

        info!(%id, %token, "session created");
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
    fn set_user() {
        let mut reg = SessionRegistry::new();
        let id = reg.create_session(SessionToken::new("t"));
        assert!(reg.get(id).unwrap().user.is_none());

        reg.set_user(id, "alice".to_string());
        assert_eq!(reg.get(id).unwrap().user.as_deref(), Some("alice"));
    }
}
