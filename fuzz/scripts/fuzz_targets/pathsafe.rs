//! Fuzz target `pathsafe` (M4b; body: Task 20): validating an untrusted file or folder name
//! from a library (HV-SCRIPT-03 step 4) never panics, whatever the bytes, and joining a path
//! never yields a path outside the library root.
//!
//! Input: UTF-8 lines. The first is a name for `pathsafe::component`; the second a
//! library-relative path for `RelPath::parse` and `pathsafe::join_with`; every further line
//! describes the in-memory tree the join walks: `D<rel>` a folder, `F<rel>` a file,
//! `L<rel>=<target>` a link with that text (relative, absolute, `..`, anything).
//!
//! Oracles (Task 20):
//! - an accepted name is one component: not empty, not `.`/`..`, no `/`, `\`, NUL or control
//!   character, at most `MAX_NAME_BYTES`, and a `RelPath` of its own;
//! - a join that answers holds no `.`/`..` and stays under the fixed root, and a link it
//!   followed names a target inside the library whose every component is accepted.

#![no_main]

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use hacivat_scripts::pathsafe::{self, JoinedKind, MAX_NAME_BYTES};
use hacivat_scripts::scan::{Fs, Meta, MetaKind};
use hacivat_scripts::{LibraryId, LibraryRoot, RelPath};
use libfuzzer_sys::fuzz_target;

const ROOT: &str = "/hv-fuzz/library";

/// What is at a path of the in-memory tree.
#[derive(Clone)]
enum Node {
    Dir,
    File,
    Link(PathBuf),
}

/// An in-memory file system: the listed nodes, every folder above them, and nothing else.
struct Tree(HashMap<PathBuf, Node>);

impl Tree {
    fn node(&self, p: &Path) -> Option<Node> {
        if let Some(n) = self.0.get(p) {
            return Some(n.clone());
        }
        // The root and every folder above a listed node are folders.
        let is_parent = p == Path::new(ROOT)
            || Path::new(ROOT).starts_with(p)
            || self.0.keys().any(|k| k.starts_with(p) && k != p);
        is_parent.then_some(Node::Dir)
    }
}

impl Fs for Tree {
    fn read_dir(&self, _dir: &Path, _cap: usize) -> io::Result<Vec<OsString>> {
        Ok(Vec::new())
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<Meta> {
        let kind = match self.node(path) {
            Some(Node::Dir) => MetaKind::Dir,
            Some(Node::File) => MetaKind::File,
            Some(Node::Link(_)) => MetaKind::Symlink,
            None => return Err(io::ErrorKind::NotFound.into()),
        };
        Ok(Meta {
            kind,
            len: 1,
            mtime: SystemTime::UNIX_EPOCH,
            dataless: false,
        })
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        // Called only on link-free paths inside the root: lexically the same path.
        match self.node(path) {
            Some(Node::Link(_)) => Err(io::ErrorKind::InvalidInput.into()),
            Some(_) => Ok(path.to_path_buf()),
            None => Err(io::ErrorKind::NotFound.into()),
        }
    }

    fn read_link(&self, link: &Path) -> io::Result<PathBuf> {
        match self.node(link) {
            Some(Node::Link(t)) => Ok(t),
            _ => Err(io::ErrorKind::InvalidInput.into()),
        }
    }
}

/// The checks an accepted name passes, apart from the validator.
fn check_component(name: &str) {
    let Ok(ok) = pathsafe::component(OsStr::new(name)) else {
        return;
    };
    assert_eq!(ok, name, "an accepted name is the name itself");
    assert!(!ok.is_empty() && ok != "." && ok != "..", "{ok:?}");
    assert!(
        !ok.contains(['/', '\\', '\0']) && !ok.chars().any(char::is_control),
        "{ok:?}"
    );
    assert!(ok.len() <= MAX_NAME_BYTES, "{} bytes", ok.len());
    let rel = RelPath::parse(ok).expect("an accepted name is a RelPath");
    assert_eq!(rel.components().count(), 1, "{ok:?}");
}

fn check_join(rel_text: &str, tree: &Tree) {
    let Ok(rel) = RelPath::parse(rel_text) else {
        return;
    };
    let root = LibraryRoot {
        id: LibraryId::new(),
        path: PathBuf::from(ROOT),
        canonical: PathBuf::from(ROOT),
    };
    let Ok(joined) = pathsafe::join_with(tree, &root, &rel) else {
        return;
    };
    assert!(
        joined
            .path
            .components()
            .all(|c| !matches!(c, Component::ParentDir | Component::CurDir)),
        "a joined path holds `.` or `..`: {:?}",
        joined.path
    );
    assert!(
        joined.path.starts_with(ROOT) && joined.path != Path::new(ROOT),
        "a joined path outside the root: {:?}",
        joined.path
    );
    match &joined.target {
        Some(target) => {
            assert_eq!(joined.kind, JoinedKind::File, "only a file link has a target");
            for c in target.components() {
                assert!(pathsafe::component(OsStr::new(c)).is_ok(), "{c:?}");
            }
            assert_eq!(joined.path, Path::new(ROOT).join(target.as_str()));
        }
        None => assert_eq!(joined.path, Path::new(ROOT).join(rel.as_str())),
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let mut lines = text.split('\n');
    let name = lines.next().unwrap_or("");
    check_component(name);
    let _ = RelPath::parse(name);
    let Some(rel_text) = lines.next() else {
        return;
    };
    let mut nodes = HashMap::new();
    for line in lines.take(16) {
        let (kind, rest) = line.split_at(line.char_indices().nth(1).map_or(line.len(), |(i, _)| i));
        let node = match kind {
            "D" => Some((rest, Node::Dir)),
            "F" => Some((rest, Node::File)),
            "L" => rest
                .split_once('=')
                .map(|(at, to)| (at, Node::Link(PathBuf::from(to)))),
            _ => None,
        };
        if let Some((at, node)) = node
            && !at.is_empty()
            && !at.split('/').any(|c| c == ".." || c == "." || c.is_empty())
        {
            nodes.insert(Path::new(ROOT).join(at), node);
        }
    }
    check_join(rel_text, &Tree(nodes));
});
