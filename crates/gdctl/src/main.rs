//! gdctl is the Godello command line entry point.
//!
//! Most of Godello is meant to be usable here without ever opening a window.
//! Running gdctl with no arguments will eventually launch the iced GUI instead.
//! For now this is a placeholder scaffold.

// The real network client lives here. It is not wired into a command yet, so it
// reads as unused in a normal build. The CLI commands that use it come next.
#[allow(dead_code)]
mod net;

fn main() {
    println!("{} (gdctl) scaffold", godello_core::APP_NAME);
}
