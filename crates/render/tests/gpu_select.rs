//! Hardware checks for T1-05, ignored in GPU-free CI. Run on the owner's laptop:
//! `cargo test --release -p render --test gpu_select -- --ignored --nocapture`.

use std::process::Command;

fn run(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_gpu-select"))
        .args(args)
        .output()
        .expect("gpu-select starts");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

/// Default settings must find a GPU without any environment variable.
#[test]
#[ignore = "needs a GPU"]
fn default_selection_finds_a_hardware_adapter() {
    let (ok, text) = run(&[]);
    println!("{text}");
    assert!(ok, "selection failed or the process crashed at exit");
    assert!(
        !text.contains("\"device\": \"Cpu\""),
        "fell back to software rendering"
    );
}

/// T0-14: gpu-probe crashed at exit with GL on the Vega 10. The selection must
/// create and drop a GL device and exit cleanly. Set the NVIDIA "Integrated
/// graphics" program setting for this test binary to reproduce the T0-14 case.
#[test]
#[ignore = "needs a GPU; reproduces the T0-14 GL exit crash check"]
fn gl_device_exits_cleanly() {
    let (ok, text) = run(&["--backend", "gl"]);
    println!("{text}");
    assert!(ok, "GL selection failed or crashed at exit:\n{text}");
}
