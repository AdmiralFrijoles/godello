//! gdctl is the Godello command line entry point.
//!
//! Most of Godello is meant to be usable here without ever opening a window.
//! Running gdctl with no command will eventually launch the iced GUI. Until that
//! exists, it prints help.

mod cli;
mod commands;
mod context;
mod interaction;
mod net;
mod progress;

use clap::{CommandFactory, Parser};

use cli::Cli;
use context::Context;

#[tokio::main]
async fn main() {
    let code = run().await;
    std::process::exit(code);
}

/// Parse the arguments, run the command, and turn the result into an exit code.
/// Zero means success. One means the command reported an error.
async fn run() -> i32 {
    let cli = Cli::parse();

    let Some(command) = cli.command else {
        // No command means the desktop app, which is not built yet. Show help.
        let mut help = Cli::command();
        let _ = help.print_long_help();
        println!();
        println!("The desktop app is not built yet. Use one of the commands above.");
        return 0;
    };

    let mut ctx = match Context::load(cli.yes, cli.silent) {
        Ok(ctx) => ctx,
        Err(err) => {
            eprintln!("error: {err:#}");
            return 1;
        }
    };

    match commands::dispatch(&mut ctx, command).await {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("error: {err:#}");
            1
        }
    }
}
