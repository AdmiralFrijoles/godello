# Launcher UI study

Notes from studying three existing Godot launchers for layout and design ideas.
These are study notes only. Per the project rules we never name these tools in
code comments, the README, or any user facing output. Here in docs/research it is
fine.

## Godot Launcher (godotlauncher/launcher)

The reference the user likes most. Electron plus React, Tailwind and daisyUI,
lucide icons, Nunito Sans. Small window (about 1024 by 600).

Layout:

- Persistent left sidebar about 224 px wide, icon plus label, never icon only.
  Primary destinations at the top (Projects, Installs). A flex spacer. A bottom
  group separated by a top border (Community, Help, Settings). Settings is a
  bottom rail item, not a separate window.
- Content is full page. Lists are tables or rows, not cards and not a grid.
- When no editor is installed it shows a warning ring on the Installs nav item
  and force navigates there on launch. A clear do this first cue.

Projects view:

- One project per row. Name in bold with small status icons beside it (a C#
  pill, integration icons). A muted full path under the name with a copy button.
  A relative modified time. An editor version dropdown on the row that rebinds
  the project. A trailing three dot overflow menu for per project actions.
- Add imports an existing project.godot. Drag and drop adds one or many with a
  drop overlay and a batch progress readout. New Project opens a full pane
  subview, not a small dialog.
- When the bound editor is missing it shows an amber triangle and a required
  editor modal that lists the requested version and a compatible fallback, with
  actions to add anyway or cancel.

Installs view:

- Installed editors as rows. Version string with a .NET pill for Mono, the muted
  install path under it, a trailing overflow menu (reinstall, remove).
- Install Editor opens a modal. Inside it has two tabs, Released and Prerelease,
  with counts. A table of versions with a Released date. The download cell has
  two separate buttons side by side, one for the standard build and one for the
  .NET build. The button recolors in place to show downloading then installed.
  No blocking progress bar, the modal stays usable while several download.

Settings:

- Full page reached from the bottom gear, with a horizontal tab strip (Projects,
  Installs, Appearance, Behavior, Tools, Updates). Appearance has a theme choice
  of Light, Dark, or System and a language dropdown. Each control has a heading
  and a one line description.

Visual language:

- Flat. No shadows or gradients. Separation comes from layered backgrounds and
  thin borders. Everything rounded at about 8 px.
- Dark theme is low chroma blue grey. Soft grey text, not pure white. The accent
  is a muted Godot blue near 3C77C2. Warning is amber. The primary blue is the
  New Project and Install Editor color and the active download color.
- Page titles are large and light weight. Table headers are tiny and muted.

## Godots (MakovWait/godots)

Built in Godot itself in GDScript, so it inherits the editor theme for free.

- Single window, top tab bar (Projects, Asset Library, Editors). Editors holds a
  local and remote toggle inside one tab.
- Both lists use the same row widget. A search box with a tag: prefix syntax, a
  sort dropdown, and a right side action panel that reacts to selection. The
  action panel is a light master detail, a list on the left and the selected
  item's actions on the right.
- Project rows are rich: project icon, name over path, a favorite star, a bound
  editor chip, a warning icon when the editor is missing, tags, and a feature
  label in the warning color (this is where the C# or Mono flag shows).
- Downloads are a stacked list of progress items. Each shows a phase label
  (Resolving, Connecting, Downloading) and byte counts, then Ready to install.
- Remove uses a two step confirm with an explicit also delete from disk checkbox
  and a loud warning.
- One drag and drop target infers project versus zip versus editor binary.
- Settings is a modal grouped into config, theme, advanced, and network. Many
  settings need a restart because of how the embedded engine applies them.

## Godot Manager (eumario/godot-manager)

Built in Godot with C#. Lightly maintained now, but the UI is complete.

- Left icon rail about 64 px, icon only, with an active left edge accent bar.
  Destinations: Projects, Asset Library, Godot Versions, and a Settings gear
  pinned at the bottom. Each page has its own header with a large pill title and
  a center toolbar of small action icons.
- Versions page is two sections in one scroll, Installed above and Available to
  download below. Each row has an icon, a version title with a Mono suffix, the
  source and size, and inline action icons. Install is a green download arrow,
  remove is a red icon, per version options is a yellow key. Semantic action
  colors read well. A View C# checkbox filters to Mono and a Mirrors dropdown
  picks the source.
- Project rows show a thumbnail, name, description, path, the bound engine and
  variant as plain text, and a favorite heart. Projects group under collapsible
  categories. A list, grid, and tree view toggle exists.
- Variant is encoded both as a Mono suffix on the label and as the icon color.
- Settings is a labeled form with sub tabs (General, Projects, About, Licenses).
  It has separate install and cache locations and an editable mirror list.
- Download progress is weak or not visible. A gap to improve on.

## What to adopt for our app

Strongest shared and most useful patterns, weighted toward the favorite.

1. Left sidebar with icon plus label. Primary destinations at the top, Settings
   pinned at the bottom. We already have this. Add icons.
2. Lean toward full page row lists with inline controls and a trailing overflow
   menu, the way the favorite does, rather than a heavy detail pane.
3. Variant as a small pill or badge on the row, and consider encoding it in color
   too. This fits our first class variant decision.
4. Install flow as a modal or focused view with a stable versus prerelease split
   (tabs or a segment) and a per variant install action per row. Show progress in
   place on the row, keep the rest usable.
5. Per project row shows the bound engine version inline, with a clear needs
   install state and an offer to install when the pinned version is missing.
6. A do this first cue when no engine is installed.
7. Flat dark visual language. Layered backgrounds, thin borders, about 8 px
   corners, a muted Godot blue accent, amber for warnings, soft grey text. iced
   gives us themes and palettes, so we can match this and still offer a switcher.
8. Two step remove with an explicit also delete from disk option and a warning.
9. A stacked downloads list with phase labels and byte counts for clarity.

## What to avoid

- Bundling a whole engine to render the UI (two of these do). Our iced binary is
  far lighter, a real advantage.
- Source locked to one mirror. Keep the repository behind a trait as planned.
- No arch detection. We detect x86_64 versus arm64, which all three miss. Show it
  on version rows.
- Restart required settings. With iced we can apply theme and scale live.
- Truncated source URLs as row noise. Show a short channel label instead.
