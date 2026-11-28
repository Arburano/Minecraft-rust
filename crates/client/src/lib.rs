//! Client library: the parts of the game client that are testable without a
//! window (the binary in `main.rs` is the window, menu and render loop).
//!
//! Spec: `docs/architecture/client-menu-and-updater.md`.

pub mod bots;
pub mod edit;
pub mod embedded;
pub mod lan;
pub mod loading;
pub mod others;
pub mod packs;
pub mod remote;
pub mod session;
pub mod settings;
pub mod textures;
pub mod worlds;

/// The capabilities this client binary was compiled with (ADR-0018). The
/// client and its embedded server share the cargo features, so this is also
/// the embedded server's set (checked by a test when both are built).
#[must_use]
pub const fn capabilities() -> world::Capabilities {
    let all = world::Capabilities::all();
    if cfg!(feature = "moving_grids") {
        all
    } else {
        all.without(world::Capability::MovingGrids)
    }
}

#[cfg(test)]
mod capability_tests {
    use super::remote::{RemoteError, check_server_capabilities};
    use world::{Capabilities, Capability};

    #[test]
    fn server_capabilities_are_checked() {
        assert!(check_server_capabilities(&[], Capabilities::NONE).is_ok());
        let zones = vec!["zones".to_owned()];
        assert!(check_server_capabilities(&zones, Capabilities::all()).is_ok());
        let grids = vec!["moving_grids".to_owned()];
        let lean = Capabilities::all().without(Capability::MovingGrids);
        let Err(RemoteError::Capability(e)) = check_server_capabilities(&grids, lean) else {
            panic!("expected a capability error");
        };
        assert!(
            e.contains("moving_grids") && e.contains("built without it"),
            "{e}"
        );
        let Err(RemoteError::Capability(e)) =
            check_server_capabilities(&["warp".to_owned()], Capabilities::all())
        else {
            panic!("expected a capability error");
        };
        assert!(e.contains("\"warp\"") && e.contains("update"), "{e}");
    }
}
