//! Profiler zones (`docs/architecture/engine-core.md`, "Profiling").
//!
//! `engine_core::zone!("name")` opens a Tracy zone until the end of the
//! enclosing block. Without the `profiling` feature the macro expands to
//! nothing, so instrumented code costs exactly what it costs without zones.
//! With the feature, a zone is recorded only while a Tracy client is running
//! (`start`); otherwise it is one atomic check.
//!
//! Enable in a binary with `--features profiling` (client/server forward it to
//! `engine-core/profiling`). Cargo unifies features, so the whole build gets
//! the same `zone!`.

#[cfg(feature = "profiling")]
#[doc(hidden)]
pub use tracy_client as __tracy;

/// Keeps the Tracy client alive; drop it at the end of `main`.
#[must_use = "the profiler stops when the guard is dropped"]
pub struct Profiler {
    #[cfg(feature = "profiling")]
    _client: tracy_client::Client,
}

/// Starts the Tracy client if the `profiling` feature is on; otherwise a no-op.
pub fn start() -> Profiler {
    Profiler {
        #[cfg(feature = "profiling")]
        _client: tracy_client::Client::start(),
    }
}

/// Whether zones are compiled in.
#[must_use]
pub const fn enabled() -> bool {
    cfg!(feature = "profiling")
}

/// Opens a profiler zone named by a string literal until the end of the block.
#[cfg(feature = "profiling")]
#[macro_export]
macro_rules! zone {
    ($name:literal) => {
        let _engine_core_zone = $crate::profile::__tracy::Client::running()
            .map(|c| c.span($crate::profile::__tracy::span_location!($name), 0));
    };
}

/// Opens a profiler zone named by a string literal until the end of the block.
/// (Compiled out: the `profiling` feature is off.)
#[cfg(not(feature = "profiling"))]
#[macro_export]
macro_rules! zone {
    ($name:literal) => {};
}

#[cfg(test)]
mod tests {
    #[test]
    fn zones_compile_in_any_configuration() {
        let _profiler = super::start();
        crate::zone!("test::outer");
        {
            crate::zone!("test::inner");
        }
        assert_eq!(super::enabled(), cfg!(feature = "profiling"));
    }
}
