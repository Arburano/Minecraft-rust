//! QUIC endpoint construction: TLS 1.3, ALPN `vxl/1` (networking-protocol.md).
//!
//! The spike uses a self-signed certificate passed in by the caller; the
//! client trusts exactly that certificate. Production certificate handling
//! is out of scope for T0-11.

use std::net::SocketAddr;
use std::sync::Arc;

use quinn::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use quinn::{Endpoint, rustls};

/// ALPN protocol identifier from networking-protocol.md.
pub const ALPN: &[u8] = protocol::ALPN;

/// Errors while building a QUIC endpoint.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// TLS configuration rejected the certificate or key.
    #[error("TLS configuration: {0}")]
    Tls(#[from] rustls::Error),
    /// The TLS configuration has no QUIC-compatible initial cipher suite.
    #[error("QUIC crypto configuration: {0}")]
    Quic(#[from] quinn::crypto::rustls::NoInitialCipherSuite),
    /// Binding the UDP socket failed.
    #[error("UDP bind: {0}")]
    Io(#[from] std::io::Error),
}

/// Server endpoint presenting `cert`/`key`, requiring ALPN `vxl/1`.
///
/// Uses BBR congestion control: the T0-11 spike showed that loss-based
/// Cubic misses the "nearest 3×3 columns ≤ 1 s" budget at 2% random loss /
/// RTT 100 ms, while BBR meets it with margin and keeps delta latency near
/// the path delay (`docs/research/net-quic-spike-2026-11-19.md`).
///
/// Binds to `addr`; pass port 0 to pick a free port and read it back with
/// [`Endpoint::local_addr`].
pub fn server_endpoint(
    addr: SocketAddr,
    cert: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
) -> Result<Endpoint, Error> {
    let mut transport = game_transport();
    transport.congestion_controller_factory(Arc::new(quinn::congestion::BbrConfig::default()));
    server_endpoint_with_transport(addr, cert, key, transport)
}

/// Like [`server_endpoint`], with an explicit transport configuration
/// (e.g. to compare congestion controllers in the spike example).
pub fn server_endpoint_with_transport(
    addr: SocketAddr,
    cert: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
    transport: quinn::TransportConfig,
) -> Result<Endpoint, Error> {
    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    tls.max_early_data_size = u32::MAX;
    let quic = quinn::crypto::rustls::QuicServerConfig::try_from(tls)?;
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(quic));
    config.transport_config(Arc::new(transport));
    Ok(Endpoint::server(config, addr)?)
}

/// Client endpoint that trusts exactly `root` (the spike's self-signed
/// certificate) and offers ALPN `vxl/1`.
pub fn client_endpoint(root: CertificateDer<'static>) -> Result<Endpoint, Error> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(root)?;
    let mut tls = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let quic = quinn::crypto::rustls::QuicClientConfig::try_from(tls)?;
    let mut endpoint = Endpoint::client("127.0.0.1:0".parse().expect("valid addr"))?;
    endpoint.set_default_client_config(quinn::ClientConfig::new(Arc::new(quic)));
    Ok(endpoint)
}

/// Client endpoint for **offline mode only** (`docs/legal.md` rule 7): it
/// accepts any server certificate. The connection is encrypted, but the
/// server is not authenticated, which is acceptable only on loopback or a
/// private LAN, where offline servers are allowed to listen. Online play
/// needs real certificate verification (stage 3, with Microsoft login).
///
/// Binds `bind` (use an unspecified address of the server's family and port 0).
pub fn client_endpoint_offline(bind: SocketAddr) -> Result<Endpoint, Error> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let tls = rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyServerCert(provider)))
        .with_no_client_auth();
    let mut tls = tls;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let quic = quinn::crypto::rustls::QuicClientConfig::try_from(tls)?;
    let mut endpoint = Endpoint::client(bind)?;
    let mut config = quinn::ClientConfig::new(Arc::new(quic));
    config.transport_config(Arc::new(game_transport()));
    endpoint.set_default_client_config(config);
    Ok(endpoint)
}

/// Transport settings of the game (client and server). The initial RTT estimate is
/// lowered from QUIC's conservative 333 ms to [`INITIAL_RTT`]: a lost
/// handshake packet is then resent after ≈ 0.3 s instead of ≈ 1 s, which
/// decides the "login ≤ 5 s" and "nearest 3×3 ≤ 1 s" budgets at 2% loss
/// (T2-02). The estimate is replaced by real samples after the first round trip.
fn game_transport() -> quinn::TransportConfig {
    let mut t = quinn::TransportConfig::default();
    t.initial_rtt(INITIAL_RTT);
    t
}

/// Initial RTT estimate of game connections (see `game_transport`).
pub const INITIAL_RTT: std::time::Duration = std::time::Duration::from_millis(100);

/// Accepts any certificate, but still checks that the server holds the key
/// of the certificate it presented (handshake signatures).
#[derive(Debug)]
struct AnyServerCert(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for AnyServerCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}
