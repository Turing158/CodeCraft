//! Session and brute-force bookkeeping for the LAN web console.
//!
//! Anyone who reaches the port can try tokens, so the rules live here in pure
//! functions: a token exchange mints a short-lived session id, repeated
//! failures back the caller off by IP, and rotating the token invalidates every
//! existing session immediately.

use std::{
    collections::HashMap,
    net::IpAddr,
    time::{Duration, Instant},
};

pub(crate) const SESSION_TTL: Duration = Duration::from_secs(12 * 60 * 60);
pub(crate) const MAX_FAILED_ATTEMPTS: u32 = 5;
pub(crate) const LOCKOUT_DURATION: Duration = Duration::from_secs(30);
pub(crate) const SESSION_COOKIE: &str = "codecraft_lan";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AuthOutcome {
    Rejected,
    LockedOut,
}

#[derive(Clone, Copy, Debug, Default)]
struct FailureRecord {
    attempts: u32,
    locked_until: Option<Instant>,
}

#[derive(Debug, Default)]
pub(crate) struct LanAuthStore {
    sessions: HashMap<String, Instant>,
    failures: HashMap<IpAddr, FailureRecord>,
}

impl LanAuthStore {
    fn prune(&mut self, now: Instant) {
        self.sessions.retain(|_, expires_at| *expires_at > now);
        self.failures.retain(|_, record| {
            record
                .locked_until
                .map(|locked_until| locked_until > now)
                .unwrap_or(record.attempts > 0)
        });
    }

    pub(crate) fn is_locked_out_at(&self, address: IpAddr, now: Instant) -> bool {
        self.failures
            .get(&address)
            .and_then(|record| record.locked_until)
            .map(|locked_until| locked_until > now)
            .unwrap_or(false)
    }

    /// Records a failed token attempt and reports whether the caller is now
    /// locked out.
    pub(crate) fn record_failure_at(&mut self, address: IpAddr, now: Instant) -> AuthOutcome {
        let record = self.failures.entry(address).or_default();
        record.attempts = record.attempts.saturating_add(1);
        if record.attempts >= MAX_FAILED_ATTEMPTS {
            record.attempts = 0;
            record.locked_until = Some(now + LOCKOUT_DURATION);
            return AuthOutcome::LockedOut;
        }
        AuthOutcome::Rejected
    }

    /// Mints a session id for a caller that presented the right token.
    pub(crate) fn grant_session_at(&mut self, address: IpAddr, now: Instant) -> String {
        self.failures.remove(&address);
        self.prune(now);
        let session_id = uuid::Uuid::new_v4().simple().to_string();
        self.sessions.insert(session_id.clone(), now + SESSION_TTL);
        session_id
    }

    pub(crate) fn session_is_valid_at(&self, session_id: &str, now: Instant) -> bool {
        self.sessions
            .get(session_id)
            .map(|expires_at| *expires_at > now)
            .unwrap_or(false)
    }

    /// Drops every session, used when the token is rotated or the server stops.
    pub(crate) fn invalidate_all(&mut self) {
        self.sessions.clear();
        self.failures.clear();
    }

    pub(crate) fn session_count_at(&self, now: Instant) -> usize {
        self.sessions
            .values()
            .filter(|expires_at| **expires_at > now)
            .count()
    }

    pub(crate) fn is_locked_out(&self, address: IpAddr) -> bool {
        self.is_locked_out_at(address, Instant::now())
    }

    pub(crate) fn record_failure(&mut self, address: IpAddr) -> AuthOutcome {
        self.record_failure_at(address, Instant::now())
    }

    pub(crate) fn grant_session(&mut self, address: IpAddr) -> String {
        self.grant_session_at(address, Instant::now())
    }

    pub(crate) fn session_is_valid(&self, session_id: &str) -> bool {
        self.session_is_valid_at(session_id, Instant::now())
    }

    pub(crate) fn session_count(&self) -> usize {
        self.session_count_at(Instant::now())
    }
}

/// Reads one cookie value out of a raw Cookie header.
pub(crate) fn cookie_value(header: &str, name: &str) -> Option<String> {
    header.split(';').find_map(|entry| {
        let (key, value) = entry.split_once('=')?;
        if key.trim() == name {
            Some(value.trim().to_string())
        } else {
            None
        }
    })
}

/// Builds the session cookie. HttpOnly keeps it away from page scripts and
/// SameSite=Strict blocks cross-site requests from carrying it.
pub(crate) fn session_cookie(session_id: &str) -> String {
    format!(
        "{SESSION_COOKIE}={session_id}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}",
        SESSION_TTL.as_secs()
    )
}

pub(crate) fn cleared_session_cookie() -> String {
    format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address() -> IpAddr {
        IpAddr::from([192, 168, 1, 40])
    }

    #[test]
    fn a_granted_session_is_accepted_until_it_expires() {
        let now = Instant::now();
        let mut store = LanAuthStore::default();
        let session = store.grant_session_at(address(), now);

        assert!(store.session_is_valid_at(&session, now + Duration::from_secs(60)));
        assert!(!store.session_is_valid_at(&session, now + SESSION_TTL + Duration::from_secs(1)));
        assert!(!store.session_is_valid_at("someone-elses-session", now));
    }

    #[test]
    fn five_wrong_tokens_lock_the_caller_out_for_thirty_seconds() {
        let now = Instant::now();
        let mut store = LanAuthStore::default();

        for _ in 0..4 {
            assert_eq!(
                store.record_failure_at(address(), now),
                AuthOutcome::Rejected
            );
        }
        assert_eq!(
            store.record_failure_at(address(), now),
            AuthOutcome::LockedOut
        );
        assert!(store.is_locked_out_at(address(), now + Duration::from_secs(29)));
        assert!(!store.is_locked_out_at(address(), now + LOCKOUT_DURATION + Duration::from_secs(1)));
    }

    #[test]
    fn a_lockout_only_applies_to_the_offending_address() {
        let now = Instant::now();
        let mut store = LanAuthStore::default();
        for _ in 0..MAX_FAILED_ATTEMPTS {
            store.record_failure_at(address(), now);
        }

        assert!(!store.is_locked_out_at(IpAddr::from([192, 168, 1, 41]), now));
    }

    #[test]
    fn a_successful_exchange_clears_earlier_failures() {
        let now = Instant::now();
        let mut store = LanAuthStore::default();
        store.record_failure_at(address(), now);
        store.record_failure_at(address(), now);
        store.grant_session_at(address(), now);

        for _ in 0..4 {
            assert_eq!(
                store.record_failure_at(address(), now),
                AuthOutcome::Rejected
            );
        }
    }

    #[test]
    fn rotating_the_token_invalidates_existing_sessions() {
        let now = Instant::now();
        let mut store = LanAuthStore::default();
        let session = store.grant_session_at(address(), now);
        store.invalidate_all();

        assert!(!store.session_is_valid_at(&session, now));
        assert_eq!(store.session_count_at(now), 0);
    }

    #[test]
    fn expired_sessions_stop_counting_as_connected_clients() {
        let now = Instant::now();
        let mut store = LanAuthStore::default();
        store.grant_session_at(address(), now);

        assert_eq!(store.session_count_at(now), 1);
        assert_eq!(
            store.session_count_at(now + SESSION_TTL + Duration::from_secs(1)),
            0
        );
    }

    #[test]
    fn cookies_are_parsed_by_name_and_not_by_prefix() {
        let header = "theme=dark; codecraft_lan=abc123; other=1";

        assert_eq!(
            cookie_value(header, SESSION_COOKIE),
            Some("abc123".to_string())
        );
        assert_eq!(cookie_value(header, "missing"), None);
        assert_eq!(
            cookie_value("codecraft_lan_extra=nope", SESSION_COOKIE),
            None
        );
    }

    #[test]
    fn the_session_cookie_is_locked_down() {
        let cookie = session_cookie("abc123");

        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(cleared_session_cookie().contains("Max-Age=0"));
    }
}
