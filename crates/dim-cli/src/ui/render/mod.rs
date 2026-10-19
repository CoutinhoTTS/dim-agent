//! Output side of the terminal interfaces: scrollback and fullscreen.

pub mod fullscreen;
pub mod markdown;
/// Builds OSC 8 links for the renderer tests.
#[cfg(test)]
pub mod osc8;
pub mod panel;
pub mod screen;
#[cfg(test)]
pub mod testterm;

pub use fullscreen::FullScreen;
pub use screen::{Frame, Overlay, Screen};
