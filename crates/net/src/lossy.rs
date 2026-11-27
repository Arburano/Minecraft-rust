//! Deterministic loss/delay relay for tests and the spike example.
//!
//! A UDP socket that forwards packets between one client and the server,
//! dropping each packet independently with probability `loss` and delaying
//! every surviving packet by `one_way_delay`. This emulates loss at the
//! transport level (the task's alternative to `tc netem`, which needs root).
//!
//! This is test tooling, not engine code: per-packet allocation is
//! acceptable here. Packets keep their order (one delay line).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

/// SplitMix64 step; deterministic PRNG so loss patterns are reproducible
/// from a seed without pulling in `rand`.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A running relay. Abort it with [`LossyRelay::abort`] when the test ends.
pub struct LossyRelay {
    local_addr: SocketAddr,
    task: JoinHandle<()>,
}

impl LossyRelay {
    /// Bind a relay on loopback that forwards to `server_addr`.
    ///
    /// `loss` is the per-packet drop probability per direction (clamped to
    /// `[0, 1]`); `one_way_delay` is applied per direction, so an RTT of
    /// 100 ms means a 50 ms delay here. The first packet received defines
    /// the client address.
    pub async fn spawn(
        server_addr: SocketAddr,
        one_way_delay: Duration,
        loss: f64,
        seed: u64,
    ) -> std::io::Result<Self> {
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await?);
        let local_addr = socket.local_addr()?;
        let loss = loss.clamp(0.0, 1.0);
        let task = tokio::spawn(relay_loop(socket, server_addr, one_way_delay, loss, seed));
        Ok(Self { local_addr, task })
    }

    /// Address the client should connect to instead of the server.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Stop the relay. In-flight delayed packets are dropped.
    pub fn abort(self) {
        self.task.abort();
    }
}

async fn relay_loop(
    socket: Arc<UdpSocket>,
    server_addr: SocketAddr,
    delay: Duration,
    loss: f64,
    seed: u64,
) {
    let mut client_addr: Option<SocketAddr> = None;
    let mut rng_to_server = seed;
    let mut rng_to_client = seed ^ 0xDEAD_BEEF_CAFE_F00D;
    let mut buf = vec![0u8; 65_536];
    // One delivery queue for both directions: packets leave in the order they
    // arrived, each `delay` after its arrival. (A timer task per packet would
    // reorder packets that arrive within the same timer tick, which a plain
    // delay line — like `tc netem delay` without jitter — never does.)
    let (tx, mut rx) =
        tokio::sync::mpsc::unbounded_channel::<(tokio::time::Instant, SocketAddr, Vec<u8>)>();
    let sender = Arc::clone(&socket);
    let delivery = tokio::spawn(async move {
        while let Some((at, dest, packet)) = rx.recv().await {
            tokio::time::sleep_until(at).await;
            let _ = sender.send_to(&packet, dest).await;
        }
    });
    loop {
        let Ok((len, from)) = socket.recv_from(&mut buf).await else {
            break;
        };
        let (dest, rng) = if from == server_addr {
            match client_addr {
                Some(addr) => (addr, &mut rng_to_client),
                None => continue, // no client known yet
            }
        } else {
            client_addr = Some(from);
            (server_addr, &mut rng_to_server)
        };
        let drop_it = (splitmix64(rng) as f64 / u64::MAX as f64) < loss;
        if drop_it {
            continue;
        }
        let at = tokio::time::Instant::now() + delay;
        if tx.send((at, dest, buf[..len].to_vec())).is_err() {
            break;
        }
    }
    delivery.abort();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix_is_deterministic_and_varied() {
        let mut a = 42;
        let mut b = 42;
        let values: Vec<u64> = (0..8).map(|_| splitmix64(&mut a)).collect();
        for &v in &values {
            assert_eq!(v, splitmix64(&mut b));
        }
        assert!(values.windows(2).all(|w| w[0] != w[1]));
    }

    #[tokio::test]
    async fn relay_forwards_and_drops() {
        // A "server" that echoes nothing; we only check forwarding and loss.
        let server = UdpSocket::bind("127.0.0.1:0").await.expect("bind server");
        let server_addr = server.local_addr().expect("server addr");
        let relay = LossyRelay::spawn(server_addr, Duration::ZERO, 0.0, 7)
            .await
            .expect("spawn relay");
        let client = UdpSocket::bind("127.0.0.1:0").await.expect("bind client");

        let mut buf = [0u8; 64];
        for i in 0..50u8 {
            client
                .send_to(&[i], relay.local_addr())
                .await
                .expect("client send");
            let (len, from) = server.recv_from(&mut buf).await.expect("server recv");
            assert_eq!(&buf[..len], &[i]);
            assert_eq!(from, relay.local_addr());
            // Reply back through the relay to the (learned) client address.
            server.send_to(&[i], from).await.expect("server send");
            let (len, _) = client.recv_from(&mut buf).await.expect("client recv");
            assert_eq!(&buf[..len], &[i]);
        }

        // 100% loss: nothing arrives.
        let drop_all = LossyRelay::spawn(server_addr, Duration::ZERO, 1.0, 7)
            .await
            .expect("spawn drop relay");
        client
            .send_to(&[0xFF], drop_all.local_addr())
            .await
            .expect("client send");
        let timeout = tokio::time::timeout(Duration::from_millis(100), server.recv_from(&mut buf));
        assert!(timeout.await.is_err(), "packet must be dropped");
        relay.abort();
        drop_all.abort();
    }
}
