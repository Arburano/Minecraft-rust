//! Saturates every logical CPU with a spin loop until the process is killed.
//!
//! Load generator for the T0-13 interruption-latency measurements:
//!
//! ```text
//! cargo run -p modhost --release --example cpu_load
//! ```
//!
//! Stop it with Ctrl+C or by killing the process (on Windows:
//! `taskkill /IM cpu_load.exe /F`).

fn main() {
    let cpus = std::thread::available_parallelism().map_or(4, std::num::NonZeroUsize::get);
    eprintln!("cpu_load: spinning {cpus} threads; kill this process to stop");
    for _ in 0..cpus {
        std::thread::spawn(|| {
            loop {
                std::hint::spin_loop();
            }
        });
    }
    loop {
        std::thread::park();
    }
}
