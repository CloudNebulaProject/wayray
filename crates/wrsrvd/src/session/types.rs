use std::time::{Duration, Instant};

/// Unique identifier for a compositor session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(u64);

impl SessionId {
    pub fn from_raw(id: u64) -> Self {
        Self(id)
    }

    pub fn raw(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "session-{}", self.0)
    }
}

/// Opaque token that identifies a session across connections.
///
/// This is NOT the user's identity — it's a session locator. Different
/// token backends (software UUID, smart card, NFC) all produce these.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionToken(pub String);

impl SessionToken {
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Session lifecycle states per ADR-004.
///
/// ```text
///     [Creating] → [Active] ⇄ [Suspended] → [Destroyed]
///                      ↓                         ↑
///                  [Destroyed] ←─────────────────┘
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Session is being created (compositor starting, greeter launching).
    Creating,
    /// Session is active with a connected client.
    Active,
    /// Client disconnected; session preserved, rendering paused.
    Suspended,
    /// Session terminated; all resources cleaned up.
    Destroyed,
}

impl SessionState {
    /// Check if transitioning from this state to `next` is valid.
    pub fn can_transition_to(self, next: SessionState) -> bool {
        matches!(
            (self, next),
            (SessionState::Creating, SessionState::Active)
                | (SessionState::Creating, SessionState::Destroyed)
                | (SessionState::Active, SessionState::Suspended)
                | (SessionState::Active, SessionState::Destroyed)
                | (SessionState::Suspended, SessionState::Active)
                | (SessionState::Suspended, SessionState::Destroyed)
        )
    }
}

impl std::fmt::Display for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionState::Creating => write!(f, "creating"),
            SessionState::Active => write!(f, "active"),
            SessionState::Suspended => write!(f, "suspended"),
            SessionState::Destroyed => write!(f, "destroyed"),
        }
    }
}

/// A compositor session binding a token to a running desktop.
#[derive(Debug)]
pub struct Session {
    pub id: SessionId,
    pub token: SessionToken,
    pub state: SessionState,
    pub created_at: Instant,
    /// When the session entered Suspended state (for timeout tracking).
    pub suspended_at: Option<Instant>,
    /// How long a suspended session survives before being destroyed.
    pub suspend_timeout: Duration,
    /// Username (set after authentication).
    pub user: Option<String>,
}

impl Session {
    /// Create a new session in the Creating state.
    pub fn new(id: SessionId, token: SessionToken) -> Self {
        Self {
            id,
            token,
            state: SessionState::Creating,
            created_at: Instant::now(),
            suspended_at: None,
            suspend_timeout: Duration::from_secs(24 * 60 * 60), // 24 hours default
            user: None,
        }
    }

    /// Attempt a state transition. Returns Ok(()) if valid, Err with reason if not.
    pub fn transition(&mut self, next: SessionState) -> Result<(), SessionTransitionError> {
        if !self.state.can_transition_to(next) {
            return Err(SessionTransitionError {
                session_id: self.id,
                from: self.state,
                to: next,
            });
        }

        if next == SessionState::Suspended {
            self.suspended_at = Some(Instant::now());
        } else {
            self.suspended_at = None;
        }

        self.state = next;
        Ok(())
    }

    /// Check if a suspended session has exceeded its timeout.
    pub fn is_suspend_expired(&self) -> bool {
        if self.state != SessionState::Suspended {
            return false;
        }
        self.suspended_at
            .is_some_and(|t| t.elapsed() >= self.suspend_timeout)
    }
}

/// Error when an invalid session state transition is attempted.
#[derive(Debug)]
pub struct SessionTransitionError {
    pub session_id: SessionId,
    pub from: SessionState,
    pub to: SessionState,
}

impl std::fmt::Display for SessionTransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid session transition for {}: {} → {}",
            self.session_id, self.from, self.to
        )
    }
}

impl std::error::Error for SessionTransitionError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_lifecycle_transitions() {
        let mut session = Session::new(SessionId::from_raw(1), SessionToken::new("test-token"));

        assert_eq!(session.state, SessionState::Creating);

        // Creating → Active
        session.transition(SessionState::Active).unwrap();
        assert_eq!(session.state, SessionState::Active);
        assert!(session.suspended_at.is_none());

        // Active → Suspended
        session.transition(SessionState::Suspended).unwrap();
        assert_eq!(session.state, SessionState::Suspended);
        assert!(session.suspended_at.is_some());

        // Suspended → Active (resume)
        session.transition(SessionState::Active).unwrap();
        assert_eq!(session.state, SessionState::Active);
        assert!(session.suspended_at.is_none());

        // Active → Destroyed
        session.transition(SessionState::Destroyed).unwrap();
        assert_eq!(session.state, SessionState::Destroyed);
    }

    #[test]
    fn invalid_transitions_rejected() {
        let mut session = Session::new(SessionId::from_raw(1), SessionToken::new("t"));

        // Creating → Suspended is invalid
        assert!(session.transition(SessionState::Suspended).is_err());

        // Creating → Active is valid
        session.transition(SessionState::Active).unwrap();

        // Active → Creating is invalid
        assert!(session.transition(SessionState::Creating).is_err());

        // Destroyed is terminal
        session.transition(SessionState::Destroyed).unwrap();
        assert!(session.transition(SessionState::Active).is_err());
        assert!(session.transition(SessionState::Suspended).is_err());
    }

    #[test]
    fn creating_can_be_destroyed() {
        let mut session = Session::new(SessionId::from_raw(1), SessionToken::new("t"));
        // Creating → Destroyed (e.g., creation failed)
        session.transition(SessionState::Destroyed).unwrap();
    }

    #[test]
    fn suspend_timeout_tracking() {
        let mut session = Session::new(SessionId::from_raw(1), SessionToken::new("t"));
        session.transition(SessionState::Active).unwrap();
        session.transition(SessionState::Suspended).unwrap();

        // With default 24h timeout, should not be expired yet
        assert!(!session.is_suspend_expired());

        // Set a zero timeout to simulate expiry
        session.suspend_timeout = Duration::ZERO;
        assert!(session.is_suspend_expired());
    }
}
