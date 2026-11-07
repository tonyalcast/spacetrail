//! Arena of directory aggregates produced by a scan.
//!
//! Only directories are stored; individual files are folded into their
//! parent's totals (plus a bounded list of the largest files kept by the
//! scanner). The children of a node occupy a contiguous id range, which keeps
//! the arena compact and makes iteration cheap.

use std::ffi::{OsStr, OsString};
use std::ops::Range;
use std::path::{Path, PathBuf};

/// Index of a directory in a [`DirTree`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub(crate) u32);

impl NodeId {
    /// The root of every tree.
    pub const ROOT: Self = Self(0);

    /// Position in the arena.
    #[must_use]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Sizes and counts for a directory, either its own direct content or the
/// recursive total.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// Allocated bytes on disk (`st_blocks * 512`), hard links counted once.
    pub alloc: u64,
    /// Apparent (logical) size in bytes.
    pub apparent: u64,
    /// Non-directory entries (files, symlinks, sockets…).
    pub files: u64,
    /// Subdirectories (not counting the directory itself).
    pub dirs: u64,
    /// Cloud-only placeholder files and directories (not stored locally).
    pub dataless: u64,
    /// Logical bytes of cloud-only files (0 bytes on disk).
    pub dataless_bytes: u64,
    /// Directories that could not be read (size unknown, never assumed 0).
    pub unreadable_dirs: u64,
    /// Extra hard links skipped because their inode was already counted.
    pub hardlink_dupes: u64,
    /// Entries that vanished or failed to stat during the scan.
    pub errors: u64,
}

impl Stats {
    pub(crate) fn add(&mut self, o: &Self) {
        self.alloc += o.alloc;
        self.apparent += o.apparent;
        self.files += o.files;
        self.dirs += o.dirs;
        self.dataless += o.dataless;
        self.dataless_bytes += o.dataless_bytes;
        self.unreadable_dirs += o.unreadable_dirs;
        self.hardlink_dupes += o.hardlink_dupes;
        self.errors += o.errors;
    }
}

/// Why a directory's content is not (fully) known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DirState {
    /// Fully scanned.
    #[default]
    Complete,
    /// Listing failed with a permission error (e.g. macOS TCC without Full Disk Access).
    Unreadable,
    /// Cloud-only directory: not listed so that nothing is downloaded.
    Dataless,
    /// Listing failed for another reason (vanished, I/O error).
    Failed,
}

/// One directory in the arena.
#[derive(Debug, Clone)]
pub struct Node {
    /// Entry name (the full scan root path for the root node).
    pub name: OsString,
    /// Parent directory, `None` for the root.
    pub parent: Option<NodeId>,
    pub(crate) children: Range<u32>,
    /// The directory's own entry plus files directly inside it.
    pub own: Stats,
    /// Recursive totals, including `own`.
    pub total: Stats,
    /// Whether the content is fully known.
    pub state: DirState,
}

/// Arena of directories with contiguous children.
#[derive(Debug, Clone, Default)]
pub struct DirTree {
    pub(crate) nodes: Vec<Node>,
}

impl DirTree {
    /// Number of directories in the tree.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// `true` if the tree has no root (never the case for a finished scan).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The root node.
    ///
    /// # Panics
    /// If the tree is empty.
    #[must_use]
    pub fn root(&self) -> &Node {
        &self.nodes[0]
    }

    /// Node by id.
    #[must_use]
    pub fn get(&self, id: NodeId) -> &Node {
        &self.nodes[id.index()]
    }

    /// Ids of the direct subdirectories of `id`, in scan order.
    pub fn children(&self, id: NodeId) -> impl ExactSizeIterator<Item = NodeId> + use<> {
        self.nodes[id.index()].children.clone().map(NodeId)
    }

    /// Ids of the subdirectories of `id`, largest allocated size first.
    #[must_use]
    pub fn children_by_size(&self, id: NodeId) -> Vec<NodeId> {
        let mut ids: Vec<_> = self.children(id).collect();
        ids.sort_by(|a, b| {
            let (na, nb) = (self.get(*a), self.get(*b));
            nb.total
                .alloc
                .cmp(&na.total.alloc)
                .then_with(|| na.name.cmp(&nb.name))
        });
        ids
    }

    /// All node ids, root first, parents before children.
    pub fn ids(&self) -> impl Iterator<Item = NodeId> + use<> {
        (0..u32::try_from(self.nodes.len()).unwrap_or(u32::MAX)).map(NodeId)
    }

    /// Depth of `id` below the root (root = 0).
    #[must_use]
    pub fn depth(&self, id: NodeId) -> usize {
        let mut depth = 0;
        let mut cur = self.get(id).parent;
        while let Some(p) = cur {
            depth += 1;
            cur = self.get(p).parent;
        }
        depth
    }

    /// Full path of `id` (the root's name is the scanned root path).
    #[must_use]
    pub fn path(&self, id: NodeId) -> PathBuf {
        let mut names: Vec<&OsStr> = Vec::new();
        let mut cur = Some(id);
        while let Some(c) = cur {
            let node = self.get(c);
            names.push(&node.name);
            cur = node.parent;
        }
        names.iter().rev().collect()
    }

    /// Finds the node for `path`, which must be the root or below it.
    #[must_use]
    pub fn find(&self, path: &Path) -> Option<NodeId> {
        let root = Path::new(&self.root().name);
        let rel = path.strip_prefix(root).ok()?;
        let mut cur = NodeId::ROOT;
        for comp in rel.components() {
            let name = comp.as_os_str();
            cur = self.children(cur).find(|c| self.get(*c).name == name)?;
        }
        Some(cur)
    }
}

/// Owned directory node built by the walker before flattening into the arena.
#[derive(Debug, Default)]
pub(crate) struct BuildNode {
    pub(crate) name: OsString,
    pub(crate) own: Stats,
    pub(crate) state: DirState,
    pub(crate) children: Vec<BuildNode>,
}

impl BuildNode {
    /// Flattens the owned tree breadth-first so that every node's children
    /// get contiguous ids, computing recursive totals on the way.
    pub(crate) fn into_tree(self) -> DirTree {
        let mut totals = Vec::new();
        let mut nodes: Vec<Node> = Vec::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back((self, None::<NodeId>));
        while let Some((b, parent)) = queue.pop_front() {
            let id = u32::try_from(nodes.len()).expect("fewer than 2^32 directories");
            let first_child = id + 1 + u32::try_from(queue.len()).expect("queue fits u32");
            let n_children = u32::try_from(b.children.len()).expect("children fit u32");
            nodes.push(Node {
                name: b.name,
                parent,
                children: first_child..first_child + n_children,
                own: b.own,
                total: Stats::default(),
                state: b.state,
            });
            totals.push(b.own);
            for child in b.children {
                queue.push_back((child, Some(NodeId(id))));
            }
        }
        // Parents precede children, so folding in reverse order is a post-order sum.
        for i in (1..nodes.len()).rev() {
            let parent = nodes[i].parent.expect("non-root has a parent").index();
            let child_total = totals[i];
            totals[parent].add(&child_total);
            totals[parent].dirs += 1;
        }
        for (node, total) in nodes.iter_mut().zip(totals) {
            node.total = total;
        }
        DirTree { nodes }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(name: &str, alloc: u64) -> BuildNode {
        BuildNode {
            name: name.into(),
            own: Stats {
                alloc,
                ..Stats::default()
            },
            ..BuildNode::default()
        }
    }

    #[test]
    fn flattens_with_contiguous_children_and_totals() {
        let mut a = leaf("a", 10);
        a.children = vec![leaf("a1", 1), leaf("a2", 2)];
        let mut root = leaf("/r", 100);
        root.children = vec![a, leaf("b", 20)];
        let tree = root.into_tree();

        assert_eq!(tree.len(), 5);
        assert_eq!(tree.root().total.alloc, 133);
        assert_eq!(tree.root().total.dirs, 4);
        let a = tree.find(Path::new("/r/a")).unwrap();
        assert_eq!(tree.get(a).total.alloc, 13);
        assert_eq!(tree.get(a).total.dirs, 2);
        assert_eq!(tree.children(a).len(), 2);
        assert_eq!(
            tree.path(tree.find(Path::new("/r/a/a2")).unwrap()),
            Path::new("/r/a/a2")
        );
        assert_eq!(tree.depth(tree.find(Path::new("/r/a/a1")).unwrap()), 2);
        let by_size: Vec<_> = tree
            .children_by_size(NodeId::ROOT)
            .into_iter()
            .map(|id| tree.get(id).name.clone())
            .collect();
        assert_eq!(by_size, ["b", "a"]);
        assert!(tree.find(Path::new("/r/zzz")).is_none());
        assert!(tree.find(Path::new("/elsewhere")).is_none());
    }
}
