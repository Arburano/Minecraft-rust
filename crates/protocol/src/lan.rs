//! LAN discovery (T2-08): an offline server on a private network announces
//! itself with a small UDP broadcast; clients on the same network list it.
//!
//! Only offline servers bound to a private LAN address announce
//! (`docs/legal.md` rule 7). The packet is plain text, one line:
//! `VXEN-LAN <protocol version> <game port> <server name>`.

/// UDP port the announcements go to.
pub const LAN_PORT: u16 = 47_778;
/// Seconds between announcements.
pub const LAN_INTERVAL_SECS: u64 = 2;
/// Longest server name in an announcement, in bytes.
const MAX_NAME: usize = 64;
const MAGIC: &str = "VXEN-LAN";

/// One announcement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanAnnouncement {
    /// The server's protocol version (clients show other versions greyed out).
    pub protocol: u32,
    /// The game (QUIC) port.
    pub port: u16,
    /// Display name.
    pub name: String,
}

impl LanAnnouncement {
    /// The packet.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut name: String = self.name.chars().filter(|c| !c.is_control()).collect();
        while name.len() > MAX_NAME {
            name.pop();
        }
        format!("{MAGIC} {} {} {name}", self.protocol, self.port).into_bytes()
    }

    /// Parses a packet; `None` for anything else (never panics).
    #[must_use]
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > 256 {
            return None;
        }
        let text = std::str::from_utf8(bytes).ok()?;
        let mut parts = text.splitn(4, ' ');
        if parts.next()? != MAGIC {
            return None;
        }
        let protocol = parts.next()?.parse().ok()?;
        let port = parts.next()?.parse().ok()?;
        let name = parts.next().unwrap_or("").to_owned();
        if name.len() > MAX_NAME || name.chars().any(char::is_control) || port == 0 {
            return None;
        }
        Some(Self {
            protocol,
            port,
            name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_garbage() {
        let a = LanAnnouncement {
            protocol: 3,
            port: 47_777,
            name: "Alex's world".into(),
        };
        assert_eq!(LanAnnouncement::parse(&a.encode()), Some(a));
        for bad in [
            &b"VXEN-LAN x 1 n"[..],
            b"VXEN-LAN 3 0 n",
            b"HELLO 3 47777 n",
            b"VXEN-LAN 3",
            &[0xff, 0xfe][..],
        ] {
            assert_eq!(LanAnnouncement::parse(bad), None);
        }
        let long = LanAnnouncement {
            protocol: 3,
            port: 1,
            name: "x".repeat(200),
        };
        assert_eq!(
            LanAnnouncement::parse(&long.encode()).map(|a| a.name.len()),
            Some(64)
        );
    }
}
