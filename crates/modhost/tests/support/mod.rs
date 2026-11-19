use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

pub fn hello_component() -> anyhow::Result<&'static Path> {
    static COMPONENT: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    let result = COMPONENT.get_or_init(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("target"));
        let target = if target.is_absolute() {
            target
        } else {
            root.join(target)
        }
        .join("guest");
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let status = Command::new(cargo)
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", &target)
            .env("CARGO_PROFILE_RELEASE_PANIC", "abort")
            .args([
                "build",
                "--locked",
                "-p",
                "hello-mod",
                "--target",
                "wasm32-wasip2",
                "--release",
            ])
            .status()
            .map_err(|error| error.to_string())?;
        if !status.success() {
            return Err(
                "hello component build failed; install the pinned wasm32-wasip2 target".into(),
            );
        }
        Ok(target.join("wasm32-wasip2/release/hello_mod.wasm"))
    });
    result
        .as_deref()
        .map_err(|error| anyhow::anyhow!("{error}"))
}
