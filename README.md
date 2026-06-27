# Godello

Install, manage, and launch any version of the Godot Engine, and keep your
projects pinned to the version they need. Works on Linux, Windows, and Mac.

Godello runs as a single binary named gdctl. Use it from the command line for
everything, or run it with no subcommand to open the desktop app.

Status: the command line core works. The desktop app has started. It can list,
install, and remove engines today. The projects and settings screens are next.

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

Engines:

```sh
gdctl install 4.3                 # install the newest 4.3 standard build
gdctl install 4.4-rc1 --variant mono
gdctl install 4.3 -m              # -m is shorthand for --variant mono
gdctl list                        # what is installed
gdctl list --remote --pre         # what is available, including prereleases
gdctl search 4.3
gdctl open 4.3                     # open the editor with no project (takes --detached too)
gdctl remove 4.3
```

Projects:

```sh
gdctl project add path/to/game
gdctl project pin path/to/game 4.3   # write the required version into project.godot
gdctl edit                           # open the editor for the project in this folder
gdctl edit --no-build                # skip the C# build for this launch
gdctl run                            # run the project in this folder
gdctl run --detached                 # launch detached and return right away (or --attached)
gdctl project status path/to/game    # branch, sync state, local changes
gdctl project update path/to/game    # bring it up to date with its remote
gdctl clone https://example.com/game.git
```

When a command needs an engine version that is not installed, Godello offers to
install it first. Add --yes (or --non-interactive) to any command to skip prompts
and take the safe default, which is handy in scripts and CI.

Add --silent (-s) to any command to suppress normal output. Errors are still
shown on stderr and the exit code still tells you what happened, so it suits a
script that only cares whether a command worked. Silent also turns off prompts
and takes the safe default for each one.

Settings:

```sh
gdctl settings list                  # every setting and its current value
gdctl settings get default_variant
gdctl settings set build_csharp_before_launch false
```

Running gdctl with no arguments opens the desktop app.

## License

MIT
