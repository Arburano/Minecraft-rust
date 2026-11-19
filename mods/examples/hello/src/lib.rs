//! Example component exercising batched events and transactional block writes.

#![cfg_attr(target_arch = "wasm32", no_std)]

#[cfg(target_arch = "wasm32")]
mod allocator;

#[cfg(target_arch = "wasm32")]
mod guest {
    extern crate alloc;
    use alloc::{vec, vec::Vec};

    #[global_allocator]
    static ALLOCATOR: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

    #[panic_handler]
    fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
        core::arch::wasm32::unreachable()
    }
    use mod_sdk::engine::api::{components, host_log, types::*, world_access};
    use mod_sdk::exports::engine::api::guest::{Guest, ModDecl, SystemDecl};

    struct Hello;

    fn changes(state: u16) -> Vec<BlockChange> {
        (0..4_000)
            .map(|i| BlockChange {
                pos: BlockPos {
                    x: i % 16,
                    y: i / 256,
                    z: (i / 16) % 16,
                },
                state,
            })
            .collect()
    }

    impl Guest for Hello {
        fn init() -> ModDecl {
            let _ = components::register("hello:events", false);
            ModDecl {
                systems: [
                    "empty",
                    "set-blocks",
                    "interrupted",
                    "trap",
                    "invalid",
                    "replace-blocks",
                ]
                .into_iter()
                .map(|name| SystemDecl {
                    name: name.into(),
                    phase: Phase::Simulate,
                    reads: Vec::new(),
                    writes: Vec::new(),
                    every_n_ticks: 1,
                })
                .collect(),
                subscribes: vec![EventKind::Message],
            }
        }

        fn on_events(events: Vec<Event>) {
            let count = events.iter().fold(0u64, |sum, event| {
                sum + match event {
                    Event::Message((channel, payload)) => {
                        channel.len() as u64 + payload.len() as u64
                    }
                    _ => 1,
                }
            });
            let _ = components::set(0, &[(0, count.to_le_bytes().to_vec())]);
        }

        fn run_system(index: u32) {
            match index {
                0 => {}
                1 => {
                    let _ = world_access::set_blocks(&changes(1));
                }
                2 => {
                    let _ = world_access::set_blocks(&changes(1));
                    loop {
                        core::hint::spin_loop();
                    }
                }
                3 => {
                    let _ = world_access::set_blocks(&changes(1));
                    panic!("intentional example trap");
                }
                4 => {
                    let _ = world_access::set_blocks(&changes(1));
                    let invalid = BlockChange {
                        pos: BlockPos { x: 16, y: 0, z: 0 },
                        state: 1,
                    };
                    if world_access::set_blocks(&[invalid]).is_err() {
                        host_log::log(host_log::Level::Debug, "out-of-region batch rejected");
                    }
                }
                5 => {
                    let _ = world_access::set_blocks(&changes(2));
                }
                _ => {}
            }
        }
    }

    mod_sdk::export!(Hello with_types_in mod_sdk);
}
