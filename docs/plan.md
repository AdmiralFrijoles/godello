# Godello implementation plan

This is the plan for building Godello. It reflects three decisions:

1. A project records its required engine version inside its own project.godot file
   under a godello section.
2. Engine versions and binaries come from GitHub directly, but behind a generic
   repository abstraction so other sources can be added later.
3. The command line core ships first. The desktop app is built on top of it after.

## Goals recap

- Install and remove any version of the Godot engine.
- Keep many versions installed at once.
- Add projects and bind each to an engine version.
- Let a project require a version. If it is missing, offer to install it.
- For C# projects, build the solution before opening the editor. On by default,
  toggleable in settings.
- Optional git integration per project. Show the branch and whether it is up to
  date, bring it up to date safely, and clone a project from a repo.
- Run on Linux, Windows, and Mac.

## Crate layout

```
crates/
  godello-core/   all shared logic, no UI
  gdctl/          the gdctl binary: CLI now, desktop app later
```

The core crate holds everything with real behavior. The gdctl crate stays thin
and handles argument parsing, output, and later the desktop screens. Both share
one source of truth.

## Core modules (godello-core)

### version

Defines the Godot version type. Godot tags are not plain semver, for example
4.3-stable or 4.0-rc1. We model:

- A pattern type with optional major, minor, patch, and an optional release stage.
  Missing parts act as wildcards. Used for matching a requirement like 4.3 against
  installed builds.
- A resolved type where every part is known.
- A variant value: Standard or Mono. This is a first class value, not a bool, so
  more variants could be added.
- Stage ordering for sorting: stable, then rc, then beta, then dev, then alpha.
  Newest first.
- Two string forms. A remote form that drops trailing zero patches the way the
  tags do, and a pinned form that is fully written out. Parsing accepts both.

### repository

The abstraction for where engine versions come from. A trait so GitHub is just
one implementation.

```
trait EngineRepository {
    fn id(&self) -> &str;
    async fn list_versions(&self, include_pre: bool) -> Result<Vec<Release>>;
    async fn resolve(&self, want: &VersionPattern, variant: Variant) -> Result<Release>;
    async fn asset(&self, release: &Release, target: Target) -> Result<Asset>;
}
```

A Release is a version plus the variants and platforms it offers. An Asset is a
download url plus a checksum. A Target is the host OS, arch, and variant.

First implementation: github.

- Version list from the Godot website manifest versions.yml.
- Binaries from the godotengine/godot-builds releases by tag.
- Asset chosen by matching the filename suffix to the target, for example
  linux.x86_64, win64.exe, macos.universal.
- We detect arch ourselves (x86_64 or arm64), which the existing launchers skip.
- Checksums read from the release SHA512SUMS file when present.

A small registry holds the set of repositories. Default is github. Config can add
or select others later.

### platform

Host detection and platform specific paths.

- Detect host OS and arch with cfg macros.
- Resolve the engine executable inside an install. Windows picks the exe or the
  console exe. Mac finds the app bundle then the binary in Contents/MacOS. Linux
  matches the Godot binary by name and arch suffix.
- Resolve app data directories with the directories crate.

### install

Manages installed engines on disk.

Layout. Engines live under the engine_install_dir setting, which defaults to the
per platform base described in the config module. The cache lives under the
standard data dir.

```
<engine_install_dir>/<variant>/<version>/   the extracted engine
<data_dir>/cache/downloads/                 downloaded archives
<data_dir>/cache/manifest.json              cached version list with a refresh time
```

- Download to a partial file, verify the checksum, then rename into place.
- Extract the zip, strip a single common top folder except for app bundles, guard
  against parent path traversal, and keep Unix mode bits so the binary stays
  runnable.
- List installed versions by reading the engines tree.
- Remove a version by deleting its folder.

### project

Reads and tracks projects.

- Parse project.godot as an INI style file. Read the project name, the top level
  config_version to tell Godot 3 from Godot 4, and the godello section version
  pin.
- Detect C#. A dotnet section in project.godot for Godot 4, or a mono section for
  Godot 3, or a csproj or sln next to the project.
- Write the required version into the godello section of project.godot. This is
  the pin and it travels with the project.
- Keep a list of projects the user has added in app data. The list stores the
  project path and a cached name. The version requirement is always read from
  project.godot so it stays correct.

### csharp

- Decide if a project is a C# project using the project module signals.
- Before launching the editor for a C# project, build the solutions. Two tools are
  supported and chosen by the csharp_build_tool setting.
- Godot, the default. Run the editor for the project version with --path, then
  --build-solutions, then --quit. The build flag implies editor mode and needs a
  valid project, so headless is not used.
- dotnet. Run dotnet build on the solution, or the project file if there is no
  solution. When neither exists there is nothing to build.
- Skip the whole step when the build_csharp_before_launch setting is off.

### launch

- Launch the editor for a project, or run the project without the editor.
- Open the editor for a given version with no project. This shows the Godot
  project manager for that engine, the same as starting Godot on its own.
- Resolve the project pin to an installed engine. If none matches, return a clear
  error the caller can turn into an install prompt.
- Start attached by default so the command waits for the editor to close. The
  launch_detached setting starts it detached so the command returns right away.

### vcs and git

Optional version control integration for projects. The vcs module holds the
generic VersionControl trait and shared types, with names that are not tied to git
so other systems can be added later. Its concepts were informed by studying git,
Mercurial, Subversion, Fossil, Jujutsu, Perforce, and Lore. See
docs/research/version-control-systems.md.

Git is the first implementation, in the git module. It uses the system git command
through the same command runner used for builds, so it is a clean no op when a
project is not a git repo or when git is not installed.

Status reads, all safe and read only:

- Current branch. Report the checked out branch, or note a detached head.
- Up to date check. Compare the branch to its upstream tracking branch and report
  ahead, behind, up to date, or no upstream. An optional fetch first makes the
  answer current.
- Dirty check. Report whether the working tree has uncommitted changes.

Changing actions:

- Bring up to date. Fast forward the branch to its upstream. Abort with a clear
  message and change nothing when the working tree is dirty or the update is not a
  fast forward, for example on a conflict or diverged history.
- Reset to upstream. A separate, explicit action that hard resets the branch to its
  upstream. This loses local changes and local commits, so it always warns first
  and is never done as part of a normal update.
- Clone. Clone a repo url into a folder, then read its project.godot and add it as
  a project.

### config

App settings stored as toml in the app config dir. The config dir comes from the
directories crate, so it lands in the normal place per platform. Linux uses the
config home such as ~/.config/godello. Windows uses the Roaming AppData folder.
Mac uses Application Support.

Settings for milestone 1:

- engine_install_dir. Where engines are installed. Defaults to a sensible per
  platform location described below. The user can change it. On change, existing
  installs stay where they are and new installs use the new path. Listing scans
  the current path.
- build_csharp_before_launch. Build the C# solution before opening the editor.
  Default on.
- csharp_build_tool. Which tool builds the C# solution, godot or dotnet. Default
  godot.
- include_prereleases. Whether remote version lists include rc, beta, and dev by
  default. Default off.
- default_variant. Standard or Mono when a command does not say. Default Standard.
- launch_detached. Start the editor or project detached so the command returns
  right away. When off the command waits for the editor to close. Default off.

More settings get added as features land. Each setting has a default, so a missing
or fresh config file still works.

#### Default paths per platform

We resolve these with the directories crate so they match what users expect.
Engines can be large, so on Windows we use the local AppData path rather than the
roaming one to avoid sync of big files.

- Linux: the data home, such as ~/.local/share/godello.
- Windows: the local app data path, such as
  C:\Users\<name>\AppData\Local\godello.
- Mac: the Application Support path, such as
  ~/Library/Application Support/godello.

Engines install under an engines folder inside that base, so the full path is the
base plus engines, then variant, then version. The cache folder sits next to it.
The engine_install_dir setting overrides the base for engines only. The cache and
config stay in their standard locations.

### error

- One error type for the crate using thiserror. The gdctl crate maps these to exit
  codes and messages.

## gdctl commands (milestone 1)

```
gdctl install <version> [--variant mono]   install an engine
gdctl remove <version> [--variant mono]    remove an engine
gdctl list                                 list installed engines
gdctl list --remote [--pre]                list versions available to install
gdctl search <text>                        search available versions
gdctl open <version> [--variant mono]      open the editor for a version, no project

gdctl project add <path>                   add a project, read its pin
gdctl project list                         list added projects
gdctl project pin <path> <version>         write the required version pin
gdctl project edit <path>                  open the project in its editor
gdctl project run <path>                   run the project without the editor
gdctl project remove <path>                forget a project
gdctl project status <path>                show branch, ahead or behind, dirty state
gdctl project update <path>                fast forward to upstream, abort on conflict
gdctl project update <path> --reset        hard reset to upstream, warns, loses changes
gdctl clone <url> [dir]                    clone a repo and add it as a project

gdctl run                                  use the project.godot in the current dir
gdctl edit                                 same, but open the editor

gdctl settings get <key>
gdctl settings set <key> <value>
```

When edit or run needs a version that is not installed, gdctl prints the missing
version and asks to install it, then continues.

### Non interactive mode

Every command must be usable without prompts. A global option, such as --yes or
--non-interactive, turns off all prompts so a command can run in a script or a CI
job. This is a command option, not a setting.

With the option set, a command takes the safe default for each prompt and never
waits for input. A prompt that would install a missing version proceeds. A prompt
that could lose data, such as a git reset, does not happen unless the action was
asked for explicitly, for example with --reset. When a command cannot continue
without a real choice, it fails with a clear message and a non zero exit code
rather than waiting.

The desktop app is milestone 2. Running gdctl with no command will open it once it
exists. Until then it prints help.

## Proposed dependencies

To be pinned when first used.

- clap with derive for the CLI.
- reqwest with rustls for downloads.
- tokio as the async runtime.
- serde and serde_json for metadata and caches.
- a maintained yaml crate for versions.yml.
- toml for settings.
- a tolerant ini reader for project.godot.
- zip for extraction.
- sha2 for checksum checks.
- directories for app paths.
- thiserror for the core error type, anyhow in the binary.
- indicatif for download progress in the CLI.
- the system git command on the path for the optional git features. This is an
  external tool, not a crate.

## Milestones

Milestone 1, command line core.

- version, repository with github, platform, install, project, csharp, launch,
  vcs with a git implementation, config, error.
- gdctl commands above, including the git commands and the non interactive option.
- Tests for version parsing and matching, asset selection per target, project.godot
  parsing, and the install layout.

Milestone 2, desktop app.

- An app built on the same core. Project list, installed and remote version lists,
  install flow, pin and edit, settings, and the offer to install a missing version.

Milestone 3, polish.

- Tags and favorites, manifest cache refresh, file associations, and a self update
  check. Optional extra repositories.

## Open points to revisit

- How much of the Godot website manifest we cache versus query live.
- Download progress reporting in the CLI. The downloader trait has no progress
  hook yet, so an install just prints before and after. A progress bar with
  indicatif can come once the trait can report bytes.

## Settled since the plan was written

- App data paths per OS are resolved with the directories crate. See the config
  module.
- Git uses the system git command, run through the shared command runner. It is a
  clean no op when git is not installed or the folder is not a repo. A bundled
  library was not needed.
