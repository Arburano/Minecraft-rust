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
