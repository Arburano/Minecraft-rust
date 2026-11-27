//! Finding and addressing servers (T2-08): LAN discovery and address
//! parsing for the multiplayer menu.
//!
//! Offline servers on a private network broadcast `protocol::lan`
//! announcements; the listener collects the ones heard recently.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

use protocol::lan::{LAN_INTERVAL_SECS, LAN_PORT, LanAnnouncement};

/// The default game port.
pub const DEFAULT_PORT: u16 = 47_777;
/// A server not heard for this long drops off the list.
const FORGET_AFTER: Duration = Duration::from_secs(3 * LAN_INTERVAL_SECS + 1);

/// A server heard on the LAN.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanServer {
    /// Its game address (the sender's IP and the announced port).
    pub addr: SocketAddr,
    /// Display name.
    pub name: String,
    /// Whether it speaks this build's protocol.
    pub compatible: bool,
}

/// Listens for LAN announcements (non-blocking; call [`LanListener::poll`]
/// every frame while the menu is open).
pub struct LanListener {
    socket: UdpSocket,
    heard: HashMap<SocketAddr, (LanServer, Instant)>,
}

impl LanListener {
    /// Listens on the LAN port. Fails if another program uses it (then the
    /// menu simply shows no LAN servers).
    pub fn start() -> std::io::Result<Self> {
        Self::on_port(LAN_PORT)
    }

    /// Listens on `port` (tests use a free one).
    pub fn on_port(port: u16) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(("0.0.0.0", port))?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            heard: HashMap::new(),
        })
    }

    /// Reads what arrived and drops servers not heard recently.
    pub fn poll(&mut self) {
        let mut buf = [0u8; 512];
        while let Ok((n, from)) = self.socket.recv_from(&mut buf) {
            let Some(a) = LanAnnouncement::parse(&buf[..n]) else {
                continue;
            };
            // Only private and loopback senders (legal.md rule 7).
            if !private(from.ip()) {
                continue;
            }
            let addr = SocketAddr::new(from.ip(), a.port);
            self.heard.insert(
                addr,
                (
                    LanServer {
                        addr,
                        name: a.name,
                        compatible: a.protocol == protocol::PROTOCOL_VERSION,
                    },
                    Instant::now(),
                ),
            );
        }
        self.heard.retain(|_, (_, t)| t.elapsed() < FORGET_AFTER);
    }

    /// Servers heard recently, by name.
    #[must_use]
    pub fn servers(&self) -> Vec<LanServer> {
        let mut v: Vec<LanServer> = self.heard.values().map(|(s, _)| s.clone()).collect();
        v.sort_by(|a, b| a.name.cmp(&b.name).then(a.addr.cmp(&b.addr)));
        v
    }
}

fn private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Parses `host`, `host:port`, `[v6]:port` or an IP; the default port is
/// 47777. Host names are resolved (the first address wins).
pub fn resolve(text: &str) -> Result<SocketAddr, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("Enter a server address".into());
    }
    if let Ok(a) = t.parse::<SocketAddr>() {
        return Ok(a);
    }
    if let Ok(ip) = t.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, DEFAULT_PORT));
    }
    let with_port = if t
        .rsplit_once(':')
        .is_some_and(|(_, p)| p.parse::<u16>().is_ok())
    {
        t.to_owned()
    } else {
        format!("{t}:{DEFAULT_PORT}")
    };
    with_port
        .to_socket_addrs()
        .map_err(|e| format!("Cannot find {t}: {e}"))?
        .next()
        .ok_or_else(|| format!("Cannot find {t}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(
            resolve("127.0.0.1").expect("ip"),
            "127.0.0.1:47777".parse().expect("a")
        );
        assert_eq!(
            resolve(" 192.168.1.5:5000 ").expect("ip"),
            "192.168.1.5:5000".parse().expect("a")
        );
        assert_eq!(
            resolve("[::1]:7").expect("v6"),
            "[::1]:7".parse().expect("a")
        );
        assert_eq!(resolve("localhost").expect("name").port(), DEFAULT_PORT);
        assert!(resolve("").is_err());
        assert!(resolve("no.such.host.invalid").is_err());
    }

    #[test]
    fn hears_announcements_and_ignores_garbage() {
        let mut l = LanListener::on_port(0).expect("bind");
        let port = l.socket.local_addr().expect("addr").port();
        let tx = UdpSocket::bind("127.0.0.1:0").expect("bind");
        let a = LanAnnouncement {
            protocol: protocol::PROTOCOL_VERSION,
            port: 47_777,
            name: "Den".into(),
        };
        tx.send_to(&a.encode(), ("127.0.0.1", port)).expect("send");
        tx.send_to(b"garbage", ("127.0.0.1", port)).expect("send");
        let old = LanAnnouncement {
            protocol: 1,
            port: 5,
            name: "Old".into(),
        };
        tx.send_to(&old.encode(), ("127.0.0.1", port))
            .expect("send");
        std::thread::sleep(Duration::from_millis(100));
        l.poll();
        let s = l.servers();
        assert_eq!(s.len(), 2, "{s:?}");
        assert_eq!(s[0].name, "Den");
        assert!(s[0].compatible);
        assert_eq!(s[0].addr, "127.0.0.1:47777".parse().expect("a"));
        assert!(!s[1].compatible, "another protocol version");
    }
}
