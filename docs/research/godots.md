# Godots research notes

Repo: https://github.com/MakovWait/godots
License: MIT (patterns reusable).
A Godot project manager written in GDScript and run as a Godot 4 app itself.

## Feature set

- Install multiple official editor versions side by side (stable, rc, beta, dev).
- Register custom or local binaries by pointing at an existing Godot executable.
- Bind a specific engine version to each project. This is the core feature.
- Add or import existing projects, create new projects, clone from VCS, duplicate.
- Tags on projects and editors, favorites, search and filter.
- Per project and per editor custom commands (run configs) plus extra CLI args.
- Asset Library browser. Self updater. HiDPI, theming, proxy.

## Engine discovery and download

- Version list (manifest) comes from the Godot website data file
  versions.yml at raw.githubusercontent.com/godotengine/godot-website. Each
  version has a name and flavor plus a releases array for rc, beta, and dev.
- Binaries come from the godotengine/godot-builds GitHub releases REST API at the
  tag releases endpoint. It matches assets by filename suffix per OS, for example
  win64.exe.zip, macos.universal.zip, linux.x86_64.zip.
- No CPU arch detection. It only checks the OS feature, so there is no arm64
  versus x86_64 selection. It relies on the Mac universal build and assumes x86
  elsewhere. This is a gap we can improve.
- No first class Mono or C# download path. Asset matching targets the standard
  editor zips.

## Project management and engine binding (the important part)

Godots parses project.godot as a Godot ConfigFile (INI) and reads:

- application/config/name for the name.
- application/config/tags for tags. It also writes tags back to project.godot.
- application/config/features for features.
- top level config_version to infer 3.x versus 4.x.
- presence of a mono section as the C# detection signal.
- a godots section version_hint, an explicit engine version string Godots writes.

Binding uses two layers.

1. Hard binding (the launch target) lives in the Godots projects.cfg, keyed by the
   editor executable path.
2. Soft hint (the matcher) is a version_hint string in the form
   version, stage, and an optional mono token. Stage is one of stable, rc, beta,
   dev, alpha, pre alpha. Both editors and projects carry hints. Matching uses a
   similarity score from 0 to 26 plus equality helpers.

Editor auto selection on import sorts installed editors by version_hint
similarity. If there is no hint it falls back to config_version (3 versus 4) plus
features and mono detection.

## App data and config (Godot ConfigFile, INI, under user://)

- godots.cfg holds main settings (app paths, language, tags, window, theme, global
  custom commands, network proxy).
- editors.cfg has one section per editor, with the section name set to the
  executable path. Values include name, version_hint, tags, extra args, favorite,
  and custom commands.
- projects.cfg has one section per project with editor_path, name, icon, favorite,
  tags, and custom commands.
- Dirs are versions, downloads, updates, and cache under user://.
- Pattern: the binary path is the stable primary key. The project to editor link
  is stored as a path reference.

## UX patterns worth copying

- Local (installed) versus Remote (downloadable tree) editor tabs. The remote view
  is an expandable tree of version into releases from versions.yml.
- Import flow. Pick project.godot, auto detect name, version_hint, and
  config_version or mono, then re sort the editor dropdown so the best match is
  first. OK is gated on a godot file and an editor.
- New project flow. Pick a 3.x or 4.x handler, pick a renderer, optionally init
  Git (writes Godot standard ignore and attributes files), then write name, icon,
  and renderer to project.godot.
- Project item states. A missing bound editor shows a broken icon, is dimmed, and
  run and edit are disabled. An invalid editor shows a warning and a bind editor
  first hint. Double clicking with an invalid editor auto opens the rebind dialog.
  An optional are you sure confirmation can be enabled.

## Missing engine handling

There is no auto fetch of the exact required version. Binding is by path. If the
editor is missing, the project is flagged invalid, run and edit are disabled, and
the user is sent to the rebind dialog which is pre sorted by similarity. The user
must download and rebind by hand. This is an opportunity for Godello to auto
prompt a download of the matching version.

## C# and dotnet

There is no solution build or dotnet build step before launch. The Godot editor
handles C# builds. Mono awareness is limited to detecting the mono section or mono
token for editor matching. Godello will add the pre launch C# build.

## Asset filename conventions across versions

Godots keeps one suffix list per platform and treats an asset as a match when its
name ends with any suffix. It does no arch picking beyond a coarse 32 or 64
filter, and it finds mono only by the substring mono. The list from its source:

- Linux: _x11.64.zip, _linux.64.zip, _linux.x86_64.zip, _linux.x86_32.zip,
  _linux_x86_64.zip, _linux_x86_32.zip. Note both the dot and underscore forms.
- Mac: _osx.universal.zip, _macos.universal.zip, _osx.fat.zip, _osx32.zip,
  _osx64.zip.
- Windows: _win64.exe.zip, _win32.exe.zip, _win64.zip, _win32.zip.

Godello goes further than this. We pick by real cpu arch including arm64, and we
match mono by concrete file shapes. The full set Godello supports, gathered from
the real godot-builds assets plus the Godots list:

- Linux standard: linux.x86_64, linux_x86_64, x11.64, linux.64 and the 32 and
  arm64 equivalents.
- Linux mono: mono_linux_x86_64, mono_x11_64, mono_linux.64 and the 32 and arm64
  equivalents.
- Windows standard: win64.exe, win64 and the 32 and arm64 equivalents.
- Windows mono: mono_win64, mono_win32, mono_windows_arm64. The mono Windows zip
  has no exe token.
- Mac standard: macos.universal, osx.universal, osx.fat, osx.64, osx64, osx.32,
  osx32. One build serves every arch, and 64 is preferred over 32.
- Mac mono: mono_macos.universal, mono_osx.universal, mono_osx.fat, mono_osx.64,
  mono_osx64.

The matcher never returns a name containing mono for a standard request, which is
what makes the broad standard fragments safe. This logic lives in
crates/godello-core/src/github.rs with tests against the real 4.3 names and the
older 3.x and 2.x names.

## Takeaways for Godello

1. Binding is the editor path (source of truth) plus a fuzzy version hint (the
   matcher). Adopt the version, stage, and optional mono grammar plus a similarity
   score.
2. Parse project.godot as INI for name, tags, features, config_version, and the
   mono section.
3. Persist our own version hint into project.godot so projects describe themselves
   across machines.
4. Engine source is the godot-builds releases plus the website versions.yml.
   Improve it with arch detection and first class C# Mono selection.
5. UX wins: an invalid editor state with auto rebind on double click, an auto
   detecting import dialog with a pre sorted editor dropdown, and an expandable
   remote version tree.
6. Differentiator: a guided flow that offers to download a project required
   version when it is missing.
