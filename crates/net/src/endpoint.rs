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
pub const ALPN: &[u8] = b"vxl/1";

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
    let mut transport = quinn::TransportConfig::default();
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

/// Transport settings of the game server: BBR (see [`server_endpoint`]),
/// and no streams opened by the client except the control and events
/// streams (columns flow only server → client), so a misbehaving client
/// cannot make the server buffer unbounded stream data.
#[must_use]
pub fn game_server_transport() -> quinn::TransportConfig {
    let mut t = quinn::TransportConfig::default();
    t.congestion_controller_factory(Arc::new(quinn::congestion::BbrConfig::default()));
    t.max_concurrent_uni_streams(0u8.into());
    t.max_concurrent_bidi_streams(2u8.into());
    t.max_idle_timeout(Some(idle_timeout()));
    t.keep_alive_interval(Some(KEEP_ALIVE));
    t
}

/// A connection without any packet for this long is closed.
pub const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

fn idle_timeout() -> quinn::IdleTimeout {
    quinn::IdleTimeout::from(quinn::VarInt::from_u32(IDLE_TIMEOUT.as_millis() as u32))
}

/// Keep-alive interval (well under [`IDLE_TIMEOUT`]).
pub const KEEP_ALIVE: std::time::Duration = std::time::Duration::from_secs(5);

fn client_transport() -> Arc<quinn::TransportConfig> {
    let mut t = quinn::TransportConfig::default();
    t.max_idle_timeout(Some(idle_timeout()));
    t.keep_alive_interval(Some(KEEP_ALIVE));
    // Many concurrent column streams from the server (it keeps ≤ 16 in flight).
    t.max_concurrent_uni_streams(64u8.into());
    Arc::new(t)
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
    let mut config = quinn::ClientConfig::new(Arc::new(quic));
    config.transport_config(client_transport());
    let mut endpoint = Endpoint::client(SocketAddr::from(([127, 0, 0, 1], 0)))?;
    endpoint.set_default_client_config(config);
    Ok(endpoint)
}

/// Client endpoint for **offline mode only** (development and private LAN
/// tests, legal.md rule 7): accepts the server's certificate without checking
/// who issued it, because an offline server has only a self-signed one. The
/// TLS handshake signatures are still verified, so the connection is
/// encrypted, but not authenticated: anyone on the path could pose as the
/// server. Real servers get proper certificates with Microsoft login.
// OPEN-QUESTION: 16
pub fn client_endpoint_offline(bind: SocketAddr) -> Result<Endpoint, Error> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls = rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyCertificate(provider)))
        .with_no_client_auth();
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let quic = quinn::crypto::rustls::QuicClientConfig::try_from(tls)?;
    let mut config = quinn::ClientConfig::new(Arc::new(quic));
    config.transport_config(client_transport());
    let mut endpoint = Endpoint::client(bind)?;
    endpoint.set_default_client_config(config);
    Ok(endpoint)
}

/// Accepts any server certificate but checks handshake signatures with it.
#[derive(Debug)]
struct AnyCertificate(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
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
