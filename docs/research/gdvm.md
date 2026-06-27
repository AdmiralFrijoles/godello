# gdvm (Godot Version Manager) research notes

Repo: https://github.com/adalinesimonian/gdvm
Site: https://gdvm.io
Registry API: https://registry.gdvm.io/v1
License: GPL 3.0 or later (study only).

## Language and structure

Rust, Cargo workspace, edition 2024.
The crate gdvm is the main binary and holds all logic.
The crate shim is a tiny binary that provides the godot and godot_console
aliases by re exec of gdvm with the env var GDVM_ALIAS set.

## Version discovery and download

gdvm does not scrape GitHub or TuxFamily for engine binaries. It uses its own
hosted registry and CDN.

- GET /v1/index.json returns a list of objects with id and name. Name is the tag,
  for example 4.3 stable.
- GET /v1/releases/{id}_{name}.json returns id, name, url, and binaries.
- binaries is a map from platform key to a map from arch key to an object with
  sha512 and a list of urls. It downloads the first url.

GitHub is used only for gdvm self update, which is why a github token config
exists.

Variants are a generic string value, not a csharp bool. The platform key encodes
the variant, for example linux, linux csharp, windows csharp. Arch keys are
x86_64, x86, arm64. On Mac a universal key is preferred over an arch specific
key.

Releases are cached in cache.json with a 48 hour TTL. Commands gdvm refresh and
gdvm clear cache manage the cache.

## On disk layout (under ~/.gdvm/)

```
installs/<variant>/<version>/   for example installs/default/4.3-stable/
cache/                          downloaded zip artifacts
cache.json                      registry and release metadata cache
bin/                            shim and current_godot symlink
bin/current_godot               directory symlink to the default version
default                         text file recording the default version
config.toml                     holds the github token
data_version                    integer for the migrations framework
```

A data_version value plus an ordered migrations framework evolves the layout, for
example moving old flat installs into default and csharp subdirs.

## Cross platform handling

- Host detection via cfg macros into a host OS and host arch.
- Extraction with the zip crate. It strips a common top level dir, except for app
  bundles, guards against parent dir traversal, and preserves Unix mode bits so
  the binary stays executable.
- Executable resolution per OS. On Windows it prefers the console exe when console
  is requested. On Mac it finds the app then the binary inside Contents/MacOS. On
  Linux it matches Godot_v or the arch suffix.
- Launch. Console mode runs attached and waits. GUI mode runs detached using
  daemonize on Unix and a detached process flag via winapi on Windows. The
  default is a directory symlink.

## Project pinning

- Pin file gdvm.toml with a godot section and a version value such as
  csharp:4.3 stable. A legacy .gdvmrc plain string file is still written and read
  for back compat. Lookup walks up parent dirs.
- Auto detect from project.godot. A config_version of 4 means Godot 3.x.
  Otherwise it parses config features. A dotnet section means the csharp variant.

## CLI

install, list, run, show, link, remove, search, clear cache, refresh, use (set
global default), upgrade, pin, config (with get, set, unset, list). Built with
clap. Shared flags include include pre and refresh. The csharp flag is deprecated
in favor of the csharp prefix.

## Key dependencies

clap 4, reqwest with rustls, tokio, serde and serde_json, toml, semver, zip, sha2
with digest io, indicatif, directories, anyhow, async trait, futures util,
terminal_size with textwrap, rpassword, dotenvy. i18n via fluent bundle and unic
langid. Platform specific: daemonize on non Windows, winapi on Windows. All
versions are exact pinned.

## Version naming notes (reusable)

- GodotVersion has all components optional and acts as a wildcard matcher. A
  resolved form is separate.
- The release 2.0.4.1 is a special case that adds a fourth subpatch component.
- Trailing zero patches are dropped in remote tags (4.1.0 becomes 4.1) but kept in
  pinned strings. Two serializations exist, remote and pinned.
- Prerelease ordering is stable, then rc, then beta, then dev, then unknown.
  Sorted newest first.
- Specifier grammar is an optional registry, then an optional variant, then a
  version or keyword. A custom registry dimension already exists.
- Checksum verify auto detects SHA 256 versus SHA 512 by hex length. Downloads
  stage to a partial file, verify, then rename in place.

## Takeaways for Godello

1. The biggest choice is a self hosted registry and CDN versus hitting the
   godot-builds releases directly. Direct is simpler and needs no infra. We lean
   toward direct.
2. Variants as a generic string, encoded in the install dir and the pin
   specifier. Adopt this.
3. A data_version plus migrations for evolving the on disk layout. Adopt this.
4. An optional component version type with remote and pinned serialization handles
   Godot irregular tags cleanly.
5. Atomic download (partial, then verify, then rename) and Unix mode preserving
   extraction.
