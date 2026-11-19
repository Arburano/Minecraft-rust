//! Generated guest bindings for the engine:api 0.1 component contract.

#![no_std]

#[allow(missing_docs)]
mod bindings {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "mod-world",
        pub_export_macro: true,
    });
}

pub use bindings::*;
