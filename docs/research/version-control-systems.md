# Version control concepts across systems

Notes used to keep the VersionControl trait neutral rather than git only. We
looked at git, Mercurial, Subversion, Fossil, Jujutsu, Perforce, and Epic's Lore.
The goal is names that read naturally for git but are not wrong for the others.

## Concept mapping

Current branch or position.

- git branch, hg named branch or bookmark, Fossil branch, jj bookmark (the
  working copy is a commit, not a branch).
- Subversion has no local branch. The working copy is bound to a url.
- Perforce has no local branch. Position is the client or stream plus which
  revisions are synced.
- Lore has lightweight branches.

Tracked remote.

- git upstream tracking branch, jj remote bookmark (tracked or not).
- hg named paths such as default, repository wide not per branch.
- Subversion is bound to a single repository url, intrinsic not configured.
- Fossil has a per repository default remote url.
- Perforce binds through the server port plus the client spec.
- Lore stores a repository url per branch.

Ahead and behind.

- Only git gives real commit counts. hg can derive them from incoming and
  outgoing list output.
- Subversion and Perforce are centralized, so they cannot be ahead. They only
  have a coarse out of date state.
- Fossil and jj do not report numeric counts. jj shows divergence as a conflict.
- So counts must be optional and ahead may be impossible for some systems.

Update from the remote on a clean advance.

- git pull or fast forward, hg pull then update, Subversion update, Fossil
  update, jj git fetch then rebase, Perforce sync, Lore sync.
- Fast forward is git jargon, so we call the clean case advanced.

Discard local changes and match the remote.

- git reset hard to upstream. Most others compose it. hg update clean plus strip,
  Subversion revert plus update, Fossil revert plus clean, Perforce revert plus
  clean. Lore has sync with reset, close to a one shot.

Get a fresh copy.

- git clone, hg clone, jj git clone, Lore clone.
- Subversion checkout. Perforce has no clone in the classic flow, you make a
  client spec then sync.

Refresh knowledge of the remote without touching the working copy.

- git fetch, Fossil pull, jj git fetch. hg has incoming for inspection.
- Subversion and Perforce have no local store to fill, they just contact the
  server read only.

Dirty or uncommitted.

- git, hg, Subversion, Fossil all have it.
- Perforce means files opened for edit in a pending change. A file edited on disk
  but not opened is a separate state.
- jj has no dirty state, the working copy is always a snapshot commit.

## How this shaped our trait

- status takes contact_remote, not fetch, since not every system fetches.
- the tracked remote field is tracked_remote, not upstream.
- ahead and behind became a SyncState with optional counts, and a NoRemote case.
  A centralized system simply never returns Ahead.
- the clean advance outcome is Advanced, not FastForwarded. The block reason for a
  non advance is Diverged.
- the force action is reset_to_remote, not reset_to_upstream.
- the dirty field is has_local_changes, documented to also cover opened for edit.
- branch, clone_repo, and is_repo are kept because they read well, with doc notes
  that branch may be a bookmark, stream, or position, and clone maps to checkout
  or a client sync on some systems.

Sources studied: official docs for Mercurial, Subversion, Fossil, Jujutsu,
Perforce Helix Core, and the Lore project (github.com/EpicGames/lore, announced
June 2026).
