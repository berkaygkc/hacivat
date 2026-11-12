//! The only crate allowed to depend on GPUI. Everything else goes through this adapter.

pub mod actions;
pub mod app;
pub mod assets;
pub mod fonts;
#[cfg(feature = "bench")]
pub mod frame_loop;
pub mod gpui_surface;
pub mod keymap;
pub mod metrics;
pub mod named_rules;
pub mod platform;
#[cfg(feature = "spike-support")]
pub mod spike_support;
pub mod theme;
pub mod window;

/// Frame statistics and verdicts for the benches (the gpui-free `hacivat-bench` crate, R3).
#[cfg(feature = "bench")]
pub use hacivat_bench as frames;
