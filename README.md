# Godello

Install, manage, and launch any version of the Godot Engine, and keep your
projects pinned to the version they need. Works on Linux, Windows, and Mac.

Godello runs as a single binary named gdctl. Use it from the command line for
everything, or run it with no subcommand to open the desktop app.

Status: early scaffold. Not yet usable.

## Features

- Install or remove any version of the Godot engine in one command.
- Keep many engine versions side by side.
- Add your projects and bind each one to the engine version it should use.
- Launch the editor or run a project directly, no hunting for the right binary.
- Require a specific engine version per project. If it is missing, Godello offers
  to install it.
- Build the C# solution automatically before opening a C# project (on by default,
  can be turned off in settings).

## Install

Requires a recent Rust toolchain.

```sh
cargo build --release
```

The binary is built at target/release/gdctl.

## Usage

```sh
gdctl --help
```

Run gdctl with no arguments to open the desktop app.

## License

MIT
