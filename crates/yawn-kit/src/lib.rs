//! Browser tools for Yawn applications, built only on `yawn`'s public API (`yawn::browser::global` and friends).
//!
//! The framework keeps what needs its runtime's insides; what only needs its public face lives here, so an application takes what it
//! uses and the framework stays small.

pub mod clipboard;
pub mod dismiss;
pub mod dnd;
#[cfg(feature = "editing")]
pub mod editing;
#[cfg(target_arch = "wasm32")]
mod glue;
pub mod keys;
pub mod measure;
pub mod pointer;
pub mod swipe;
pub mod viewport;
