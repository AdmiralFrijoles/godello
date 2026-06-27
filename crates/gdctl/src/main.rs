//! gdctl is the Godello command line entry point.
//!
//! Most of Godello is meant to be usable here without ever opening a window.
//! Running gdctl with no command launches the desktop app instead.
//!
//! The two paths own their async runtime differently. A command builds a tokio
//! runtime and runs the work on it. The desktop app hands control to the GUI
//! framework, which owns its own runtime. We branch before any runtime exists so
//! the two never nest, which would panic.

mod cli;
mod commands;
mod context;
mod gui;
mod interaction;
mod net;
mod progress;

use clap::Parser;

use cli::Cli;
use context::Context;

fn main() {
    std::process::exit(run());
}

/// Parse the arguments, run the command or the GUI, and turn the result into an
/// exit code. Zero means success. One means an error was reported.
fn run() -> i32 {
    let cli = Cli::parse();

    // Context::load does no async work, so we build it once here, before either
    // path starts a runtime. Both the command path and the GUI share it.
    let ctx = match Context::load(cli.yes, cli.silent) {
        Ok(ctx) => ctx,
        Err(err) => {
            eprintln!("error: {err:#}");
            return 1;
        }
    };

    match cli.command {
        // No command means the desktop app. The GUI owns the event loop and its
        // own runtime, so we hand the whole process to it and block until the
        // window closes.
        None => match gui::launch(ctx) {
            Ok(()) => 0,
            Err(err) => {
                eprintln!("error: the desktop app failed to start: {err}");
                1
            }
        },

        // A command means CLI work. We own the runtime here, so build a multi
        // thread runtime (matching the features the rest of the CLI relies on)
        // and run the dispatch on it.
        Some(command) => {
            let runtime = match tokio::runtime::Runtime::new() {
                Ok(runtime) => runtime,
                Err(err) => {
                    eprintln!("error: could not start the async runtime: {err}");
                    return 1;
                }
            };
            let mut ctx = ctx;
            runtime.block_on(async {
                match commands::dispatch(&mut ctx, command).await {
                    Ok(()) => 0,
                    Err(err) => {
                        eprintln!("error: {err:#}");
                        1
                    }
                }
            })
        }
    }
}
