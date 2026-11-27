//! Golden byte files for the handshake frames (`tests/golden/*.bin`).
//!
//! A failure here means the handshake bytes changed. If the change is
//! deliberate, bump `PROTOCOL_VERSION`, then regenerate the files with
//! `UPDATE_GOLDEN=1 cargo test -p protocol --test golden` and commit them.

mod common;

use std::path::PathBuf;

use protocol::*;

fn check(name: &str, bytes: &[u8]) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, bytes).expect("write golden file");
        return;
    }
    let want = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(
        bytes,
        want.as_slice(),
        "{name} changed; bump PROTOCOL_VERSION and regenerate (see the file header)"
    );
}

fn frame(body: &[u8]) -> Vec<u8> {
    codec::frame(body).expect("fits")
}

#[test]
fn golden_version() {
    // The golden files are for this version; bumping it requires new files.
    assert_eq!(PROTOCOL_VERSION, 1);
}

#[test]
fn hello() {
    let bytes = frame(&common::hello().encode());
    check("hello.bin", &bytes);
    assert_eq!(Hello::decode(&bytes[4..]), Ok(common::hello()));
}

#[test]
fn welcome() {
    let bytes = frame(&ServerHello::Accepted(common::welcome()).encode());
    check("welcome.bin", &bytes);
    assert_eq!(ServerHello::decode(&bytes[4..]), Ok(common::welcome()));
}

#[test]
fn rejected() {
    let bytes = frame(&ServerHello::Rejected(Rejection::OfflineNotAllowed).encode());
    check("rejected.bin", &bytes);
}

#[test]
fn version_mismatch_answer() {
    let bytes = frame(&ServerHello::encode_version_mismatch());
    check("version_mismatch.bin", &bytes);
    // The prefix layout is fixed forever: magic + u32 LE version.
    assert_eq!(&bytes[4..8], b"VXLH");
    assert_eq!(&bytes[8..12], &PROTOCOL_VERSION.to_le_bytes());
}

#[test]
fn registry() {
    let bytes = ServerControl::Registry(common::registry())
        .encode_frame()
        .expect("fits");
    check("registry.bin", &bytes);
    assert_eq!(
        ServerControl::decode(&bytes[4..]),
        Ok(ServerControl::Registry(common::registry()))
    );
}

#[test]
fn spawn() {
    let bytes = ServerControl::Spawn(common::spawn())
        .encode_frame()
        .expect("fits");
    check("spawn.bin", &bytes);
    assert_eq!(
        ServerControl::decode(&bytes[4..]),
        Ok(ServerControl::Spawn(common::spawn()))
    );
}

#[test]
fn ready() {
    check(
        "ready.bin",
        &ClientControl::Ready.encode_frame().expect("fits"),
    );
}
