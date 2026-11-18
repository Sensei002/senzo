//! Native GPUI shell and application-owned async runtime bridge.

mod artwork;
pub mod components;
mod navigation;
mod shell;
mod theme;

pub use shell::run;
