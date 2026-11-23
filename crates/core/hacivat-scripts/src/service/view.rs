// ## m4b-t10-service (fix round 1, m2)
//! A snapshot as one connection sees it: [`WorkspaceSnapshot::view`]. The workspace snapshot
//! itself holds every library's whole listing, status map and diagnostics, which name files
//! under `@<env>/` folders; those stay crate-private. Every surface that shows a library under a
//! connection (the tree, its counts and badges, the diagnostics popover, a tab's lookups) reads
//! it through a view, whose entries, counts, badges and diagnostics are filtered by the
//! connection's environment with the tree's own rule (`envscope::visible`, R-M4b-6). A
//! diagnostic that names a path not shown under the connection is never shown; it is counted
//! in [`LibraryDiag::HiddenProblems`].

use std::collections::HashSet;
use std::fmt;

use hacivat_git::{FileStatus, GitError, GitState, UntrustReason};
use hacivat_proto::EnvTag;

use super::{LibraryDiag, LibrarySnapshot, WorkspaceSnapshot};
use crate::envscope::{self, admits};
use crate::pathsafe::RelPath;
use crate::registry::LibraryId;
use crate::scan::{Diag, EntryKind, ScriptEntry};

impl WorkspaceSnapshot {
    /// The snapshot as a connection whose environment is `env` sees it (`None`: an untagged
    /// connection or an unbound tab, which sees only unscoped files).
    pub fn view(&self, env: Option<EnvTag>) -> EnvView<'_> {
        EnvView { snap: self, env }
    }
}

/// [`WorkspaceSnapshot`] filtered for one environment.
#[derive(Clone, Copy)]
pub struct EnvView<'a> {
    snap: &'a WorkspaceSnapshot,
    env: Option<EnvTag>,
}

impl<'a> EnvView<'a> {
    pub fn env(&self) -> Option<EnvTag> {
        self.env
    }

    pub fn libraries(&self) -> impl Iterator<Item = LibraryView<'a>> + 'a {
        let env = self.env;
        let snap = self.snap;
        snap.libraries.iter().map(move |l| LibraryView {
            lib: l,
            git: snap.git.get(&l.id),
            env,
        })
    }

    pub fn library(&self, id: LibraryId) -> Option<LibraryView<'a>> {
        let lib = self.snap.libraries.iter().find(|l| l.id == id)?;
        Some(LibraryView {
            lib,
            git: self.snap.git.get(&id),
            env: self.env,
        })
    }
}

// ## m4b-t10-service fix round 2 (m-b): counts only, never a path (H11).
impl fmt::Debug for EnvView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnvView")
            .field("env", &self.env)
            .field("libraries", &self.snap.libraries.len())
            .finish()
    }
}

impl fmt::Debug for LibraryView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LibraryView")
            .field("id", &self.lib.id)
            .field("env", &self.env)
            .field("state", &self.lib.state)
            .field("files", &self.file_count())
            .field("repository", &self.git())
            .finish()
    }
}

/// One library as a connection sees it.
#[derive(Clone, Copy)]
pub struct LibraryView<'a> {
    lib: &'a LibrarySnapshot,
    git: Option<&'a GitState>,
    env: Option<EnvTag>,
}

/// The repository status of a library, without its per-path map (badges come per visible
/// entry, [`LibraryView::git_status`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitSummary {
    /// No status has been computed yet (or the library is unavailable).
    Pending,
    NotARepo,
    Untrusted(UntrustReason),
    Unavailable(GitError),
    Ready,
}

impl<'a> LibraryView<'a> {
    /// The library's identity, binding and state (no entries or diagnostics: those are the
    /// view's).
    pub fn library(&self) -> &'a LibrarySnapshot {
        self.lib
    }

    /// Whether `entry` is shown under this view's connection: a file by `envscope::visible`,
    /// a folder when its own scope admits the connection.
    fn shows(&self, entry: &ScriptEntry) -> bool {
        match entry.kind {
            EntryKind::Folder => admits(&envscope::folder_scope(&entry.rel), self.env),
            EntryKind::Sql | EntryKind::Markdown => envscope::visible(entry, self.env),
        }
    }

    /// The entries shown: visible files, and the folders on their way (plus visible dataless
    /// folders, which are shown as not downloaded), in tree order.
    pub fn entries(&self) -> Vec<&'a ScriptEntry> {
        let mut holding: HashSet<&str> = HashSet::new();
        for e in self.lib.entries.iter().filter(|e| self.shows(e)) {
            if e.kind != EntryKind::Folder || e.dataless {
                let s = e.rel.as_str();
                holding.extend(s.match_indices('/').map(|(i, _)| &s[..i]));
            }
        }
        self.lib
            .entries
            .iter()
            .filter(|e| self.shows(e))
            .filter(|e| {
                e.kind != EntryKind::Folder || e.dataless || holding.contains(e.rel.as_str())
            })
            .collect()
    }

    /// The entry at `rel`, when it is shown.
    pub fn entry(&self, rel: &RelPath) -> Option<&'a ScriptEntry> {
        self.lib
            .entries
            .iter()
            .find(|e| &e.rel == rel)
            .filter(|e| self.shows(e))
    }

    /// How many files are shown (the Scripts node's count).
    pub fn file_count(&self) -> usize {
        self.lib
            .entries
            .iter()
            .filter(|e| e.kind != EntryKind::Folder && self.shows(e))
            .count()
    }

    /// The diagnostics this connection may see. One that names a path not shown under it
    /// becomes part of a [`LibraryDiag::HiddenProblems`] count, at the end.
    pub fn diagnostics(&self) -> Vec<LibraryDiag> {
        let folder_shown = |rel: &RelPath| admits(&envscope::folder_scope(rel), self.env);
        let mut out = Vec::new();
        let mut hidden = 0;
        for d in self.lib.diagnostics.iter() {
            let shown = match d {
                LibraryDiag::Scan(Diag::BadName { parent, .. }) => {
                    parent.as_ref().is_none_or(folder_shown)
                }
                LibraryDiag::Scan(Diag::Unreadable { folder, .. }) => folder_shown(folder),
                LibraryDiag::Scan(Diag::LinkScope { rel }) => {
                    admits(&envscope::scope_of(rel), self.env)
                }
                // A contradictory chain names the folders of other environments.
                LibraryDiag::Scan(Diag::EnvContradiction { .. }) => false,
                // R-M4b-6 wants the misspelt folder named ("did you mean `@production`?"): it
                // is hidden everywhere, so it names no file any connection shows.
                LibraryDiag::Scan(Diag::UnknownEnvFolder { .. })
                | LibraryDiag::Manifest(_)
                | LibraryDiag::ScanStopped(_)
                | LibraryDiag::Suppressed(_)
                | LibraryDiag::RootInEnvFolder(_)
                | LibraryDiag::RootRefused(_)
                // ## m4b-t16-attach fix round 1 (named line): a database name, no path.
                | LibraryDiag::ManifestKept(_)
                | LibraryDiag::NoLogicalName => true,
                LibraryDiag::HiddenProblems(n) => {
                    hidden += n;
                    false
                }
            };
            if shown {
                out.push(d.clone());
            } else if !matches!(d, LibraryDiag::HiddenProblems(_)) {
                hidden += 1;
            }
        }
        if hidden > 0 {
            out.push(LibraryDiag::HiddenProblems(hidden));
        }
        out
    }

    pub fn git(&self) -> GitSummary {
        match self.git {
            None => GitSummary::Pending,
            Some(GitState::NotARepo) => GitSummary::NotARepo,
            Some(GitState::Untrusted(r)) => GitSummary::Untrusted(*r),
            Some(GitState::Unavailable(e)) => GitSummary::Unavailable(e.clone()),
            Some(GitState::Ready(_)) => GitSummary::Ready,
        }
    }

    /// The badge of a shown entry (`None` for a clean, unknown or not shown path). Keyed by
    /// the scan's exact `RelPath` text (R-T1a-2: the NFD spelling macOS keeps on disk).
    pub fn git_status(&self, rel: &RelPath) -> Option<FileStatus> {
        self.entry(rel)?;
        match self.git? {
            GitState::Ready(status) => status.by_path.get(rel.as_str()).copied(),
            _ => None,
        }
    }

    /// Every badge of a shown entry, in tree order.
    pub fn git_statuses(&self) -> Vec<(&'a RelPath, FileStatus)> {
        let Some(GitState::Ready(status)) = self.git else {
            return Vec::new();
        };
        self.entries()
            .into_iter()
            .filter_map(|e| Some((&e.rel, *status.by_path.get(e.rel.as_str())?)))
            .collect()
    }
}
// ## m4b-t10-service
