//! Roundtrip app layer: Slint UI + workspace controller.

pub mod di;
pub mod modules;

// slint-generated code unwraps and todo!s internally; our own code stays linted
#[allow(clippy::unwrap_used, clippy::todo)]
pub mod ui {
    slint::include_modules!();
}
