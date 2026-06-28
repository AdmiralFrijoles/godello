# Godello

Install any version of the Godot engine, keep each project pinned to the version
it needs, and launch the editor or run a project with one click or one command.
Godello works on Linux, Windows, and macOS.

Godello is one small program named gdctl. Open it with no arguments for the
desktop app, or use it from the command line for everything the app does.

## What it does

- Install or remove any Godot version, and keep many versions side by side.
- Choose the standard build or the C# (Mono) build. Godello detects your CPU and
  downloads the right one.
- Add your projects and bind each one to the engine version it needs. The version
  travels with the project, inside its project.godot.
- Open the editor or run a project directly. No hunting for the right binary.
- If a project needs a version you do not have, Godello offers to install it.
- Build the C# solution before opening a C# project, automatically. On by
  default, and you can turn it off in settings.
- See each project's version control status at a glance, and update it from its
  remote.

## Download

Grab the latest build for your platform from the releases page:

https://github.com/AdmiralFrijoles/godello/releases

- Linux: an AppImage that runs anywhere, or a tar.gz with the binary. x86_64 and
  arm64.
- Windows: a zip with gdctl.exe. x86_64.
- macOS: a tar.gz with the binary. Apple Silicon.

On Linux, make the AppImage executable and run it:

```sh
chmod +x Godello-*.AppImage
./Godello-*.AppImage
```

On Windows and macOS, unpack the archive and run gdctl. To use it from anywhere,
put it on your PATH.

### Build from source

Godello builds with a recent Rust toolchain.

```sh
cargo build --release
```

The binary lands at target/release/gdctl.

## The desktop app

Run gdctl with no arguments to open the app. It has three screens:

- Engines: install, remove, and open any version, released or prerelease.
- Projects: add a folder or clone a repository, then edit, run, pin a version, or
  update from git.
- Settings: theme, default variant, prereleases, the C# build, and where engines
  install.

## Command line

Everything the app does is also a command. A few examples:

```sh
gdctl install 4.3                 # newest 4.3 standard build
gdctl install 4.4-rc1 --variant mono
gdctl list                        # what is installed
gdctl list --remote --pre         # what is available, with prereleases
gdctl remove 4.3
```

```sh
gdctl project add path/to/game
gdctl project pin path/to/game 4.3   # record the version in project.godot
gdctl edit                           # open the editor for the project here
gdctl run                            # run the project here
gdctl project update path/to/game    # bring it up to date with its remote
gdctl clone https://example.com/game.git
```

```sh
gdctl settings list
gdctl settings set build_csharp_before_launch false
```

Run gdctl --help for the full list. Add --yes to skip prompts and take the safe
default, which suits scripts and CI. Add --silent to quiet normal output while
still returning a meaningful exit code.

## How a project remembers its engine

Godello writes a small godello section into your project.godot, so the pin
travels with the project in version control:

```ini
[godello]
pin_version="4.3"
```

pin_version is the engine version the project needs. You can also set main_branch
to tell an update which branch to pull from, when it is not main.

## License

MIT
