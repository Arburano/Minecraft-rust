//! T2-11: the client and its embedded server agree on the compiled
//! capabilities (the client computes its set without the server crate).

#[test]
fn client_and_embedded_server_capabilities_match() {
    assert_eq!(client::capabilities(), server::capabilities::compiled());
    const {
        assert!(
            client::embedded::AVAILABLE,
            "default builds have single-player"
        )
    };
}
