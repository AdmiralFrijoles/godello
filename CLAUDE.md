# Godello

Godot engine and project launcher for Linux, Windows, and Mac. The app is named
Godello. The binary is named gdctl. Most functionality is usable from the CLI
without opening a window. Running with no subcommand will launch the iced GUI.

## Writing style rule

All comments and documentation in this project must use plain simple English with
only basic punctuation. Do not use semicolons, emdashes, hyphenated compound
words, emoji, or any other non basic punctuation. Allowed punctuation is periods,
commas, question marks, apostrophes, parentheses, and colons. Keep it short and
not verbose. This applies to code comments, these docs, the README, and commit
messages.

## Workspace layout

```
Cargo.toml                 workspace root (resolver 3, edition 2024)
crates/
  godello-core/            shared logic for engine and project management
  gdctl/                   binary gdctl: CLI and iced GUI front end
```

Keep cross platform logic with side effects (download, disk layout, project
parsing) in godello-core so both the CLI and GUI share one source of truth. Keep
the gdctl crate thin. It handles argument parsing, the GUI, and presentation.

## Core feature requirements

- Install and remove any version of the Godot engine quickly and easily.
- Add projects and bind an engine version to each so the user can launch the
  editor or run the project directly.
- A project may require a specific engine version. If no suitable version is
  installed, offer to install it.
- If a project uses the C# (Mono) build, build the C# solution before launching
  the editor. This is enabled by default and must be toggleable in settings.

## Build and test

```
cargo build
cargo test
cargo run -p gdctl -- <args>
```

## Conventions

- Engine downloads: source binaries from the godotengine/godot-builds GitHub
  releases. Version metadata can come from the Godot website versions.yml
  manifest. Detect CPU arch (x86_64 or arm64), which existing launchers miss.
- Treat the engine variant (default versus C# Mono) as a first class string
  value, not a bool. Encode it in both the install path and any pin specifier.

## Decisions

These are settled. See docs/plan.md for the full plan.

- A project records its required engine version inside its own project.godot file
  under a godello section. The pin travels with the project.
- Engine versions and binaries come from GitHub directly (godot-builds releases
  plus the website versions.yml), but behind a generic repository trait so other
  sources can be added later. Default repository is github.
- The command line core ships first. The desktop app is built on top of it after.
- Treat the engine variant (default versus C# Mono) as a first class value, not a
  bool.

## Reference launchers (researched, do not vendor)

- gdvm (adalinesimonian/gdvm, Rust, GPL 3.0): self hosted JSON registry and CDN,
  variants encoded in the platform key, installs under
  ~/.gdvm/installs/<variant>/<version>/, per project pin via gdvm.toml or
  .gdvmrc, an optional component version type with separate remote and pinned
  serialization. GPL, so study only and do not copy source.
- Godots (MakovWait/godots, GDScript, MIT): binding uses the editor path as the
  source of truth plus a fuzzy version hint for auto suggesting editors. Parses
  project.godot as INI (config_version, a mono section means C#). It has no arch
  detection and no C# pre build, which are opportunities for us. MIT, so patterns
  are reusable.

See docs/research/ for the full notes.
