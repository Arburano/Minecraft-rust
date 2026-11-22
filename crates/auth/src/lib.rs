//! Player identity and login: offline mode now; Microsoft -> Xbox Live -> XSTS ->
//! Minecraft token chain, session JWT client and secret storage after T0-10.
//!
//! Spec:
//! - `docs/architecture/auth-and-assets.md` ("Offline mode")
//! - `docs/legal.md` rule 7: offline mode is for development and private tests
//!   only, off by default, never on a publicly reachable server.
//!
//! This crate has no network dependency: offline mode cannot make Microsoft,
//! Xbox or Mojang calls by construction.

use std::fmt;

use sha2::{Digest, Sha256};

/// A validated player name: 3..=16 characters `A-Z a-z 0-9 _`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PlayerName(String);

impl PlayerName {
    /// Validates a name.
    pub fn new(name: &str) -> Result<Self, AuthError> {
        let ok = (3..=16).contains(&name.len())
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
        if ok {
            Ok(Self(name.to_owned()))
        } else {
            Err(AuthError::InvalidName(name.to_owned()))
        }
    }

    /// The name as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PlayerName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A stable 128-bit player ID in UUID layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlayerId(pub [u8; 16]);

impl PlayerId {
    /// The deterministic offline ID of a name: SHA-256 of `"offline-player:" +
    /// lowercase name`, truncated to 16 bytes, with UUID version 8 (custom) and
    /// the RFC 4122 variant. Case-insensitive, so `Alex` and `alex` are one player.
    #[must_use]
    pub fn offline(name: &PlayerName) -> Self {
        let digest = Sha256::new()
            .chain_update(b"offline-player:")
            .chain_update(name.as_str().to_ascii_lowercase().as_bytes())
            .finalize();
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        bytes[6] = (bytes[6] & 0x0f) | 0x80;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Self(bytes)
    }
}

impl fmt::Display for PlayerId {
    /// Hyphenated lowercase hex, `8-4-4-4-12`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, b) in self.0.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                f.write_str("-")?;
            }
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

/// Who a player is. Microsoft-verified identities are added once T0-10 is
/// answered (open question 4).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Identity {
    /// Development identity: a local name, no ownership check.
    Offline {
        /// Player name.
        name: PlayerName,
        /// Deterministic ID derived from the name.
        id: PlayerId,
    },
}

impl Identity {
    /// An offline identity for a name.
    pub fn offline(name: &str) -> Result<Self, AuthError> {
        let name = PlayerName::new(name)?;
        let id = PlayerId::offline(&name);
        Ok(Self::Offline { name, id })
    }

    /// The player's name.
    #[must_use]
    pub fn name(&self) -> &PlayerName {
        match self {
            Self::Offline { name, .. } => name,
        }
    }

    /// The player's ID.
    #[must_use]
    pub fn id(&self) -> PlayerId {
        match self {
            Self::Offline { id, .. } => *id,
        }
    }

    /// Whether this identity skipped the ownership check.
    #[must_use]
    pub fn is_offline(&self) -> bool {
        matches!(self, Self::Offline { .. })
    }
}

/// Which identities a server admits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AdmissionPolicy {
    /// Admit offline identities. Off by default; set only by an explicit
    /// server flag (`--offline`).
    pub allow_offline: bool,
}

impl AdmissionPolicy {
    /// Checks whether a server with this policy may admit `identity`.
    pub fn admit(&self, identity: &Identity) -> Result<(), AuthError> {
        if identity.is_offline() && !self.allow_offline {
            return Err(AuthError::OfflineNotAllowed);
        }
        Ok(())
    }
}

/// Authentication errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthError {
    /// The name is not 3..=16 characters of `A-Z a-z 0-9 _`.
    InvalidName(String),
    /// The server does not run in offline mode.
    OfflineNotAllowed,
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName(n) => {
                write!(
                    f,
                    "invalid player name {n:?}: use 3-16 letters, digits or _"
                )
            }
            Self::OfflineNotAllowed => write!(
                f,
                "this server requires a verified account; offline players are not admitted"
            ),
        }
    }
}

impl std::error::Error for AuthError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_id_is_deterministic_and_case_insensitive() {
        let a = Identity::offline("Alex_01").expect("valid");
        let b = Identity::offline("alex_01").expect("valid");
        let c = Identity::offline("Steve").expect("valid");
        assert_eq!(a.id(), b.id());
        assert_ne!(a.id(), c.id());
        assert_eq!(a.name().as_str(), "Alex_01");
        // Golden value: changing the derivation would change every saved player.
        // (Computed independently with Python hashlib.)
        assert_eq!(c.id().to_string(), "7cae3b2c-69eb-8a03-8712-d41254abbd5a");
        let s = c.id().to_string();
        assert_eq!(s.len(), 36);
        assert_eq!(&s[14..15], "8", "UUID version 8");
        assert!(
            matches!(&s[19..20], "8" | "9" | "a" | "b"),
            "RFC 4122 variant"
        );
    }

    #[test]
    fn names_are_validated() {
        for bad in ["ab", "a_very_long_name_x", "with space", "ümlaut", ""] {
            assert!(Identity::offline(bad).is_err(), "{bad:?}");
        }
        assert!(Identity::offline("abc").is_ok());
        assert!(Identity::offline("A_23456789012345").is_ok());
    }

    #[test]
    fn offline_players_need_explicit_policy() {
        let player = Identity::offline("tester").expect("valid");
        assert_eq!(
            AdmissionPolicy::default().admit(&player),
            Err(AuthError::OfflineNotAllowed)
        );
        assert_eq!(
            AdmissionPolicy {
                allow_offline: true
            }
            .admit(&player),
            Ok(())
        );
    }
}
