//! Pure tab decisions: no Slint, no IO.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabFile {
    pub path: PathBuf,
    /// root of the collection the file belongs to
    pub collection: PathBuf,
}

/// Index to activate after a close: the right neighbour, else the new last tab.
pub fn next_active(remaining: usize, closed: usize) -> Option<usize> {
    remaining.checked_sub(1).map(|last| closed.min(last))
}

/// Drop index kept inside the dragged tab's group: pinned tabs stay first.
pub fn reorder_target(pinned: &[bool], from: usize, to: usize) -> usize {
    let pins = pinned.iter().filter(|p| **p).count();
    let last = pinned.len().saturating_sub(1);
    if pinned.get(from).copied().unwrap_or(false) {
        to.min(pins.saturating_sub(1))
    } else {
        to.clamp(pins.min(last), last)
    }
}

/// Lowest free "Untitled", "Untitled 2", ….
pub fn untitled_title(taken: &[&str]) -> String {
    (1..)
        .map(|n| match n {
            1 => "Untitled".to_owned(),
            n => format!("Untitled {n}"),
        })
        .find(|t| !taken.contains(&t.as_str()))
        .unwrap_or_default()
}

/// `path` moved from under `old` to under `new`; None when it was not under `old`.
pub fn rebase_path(path: &Path, old: &Path, new: &Path) -> Option<PathBuf> {
    let rest = path.strip_prefix(old).ok()?;
    // join("") would add a trailing slash
    Some(if rest.as_os_str().is_empty() {
        new.to_path_buf()
    } else {
        new.join(rest)
    })
}

/// Session tabs whose file still exists in an open collection, plus the active index on them.
pub fn restore_tabs(
    paths: &[PathBuf],
    active: Option<usize>,
    collections: &[PathBuf],
    exists: impl Fn(&Path) -> bool,
) -> (Vec<TabFile>, Option<usize>) {
    let mut kept = Vec::new();
    let mut active_kept = None;
    for (i, path) in paths.iter().enumerate() {
        // nested collections: the deepest root owns the file
        let owner = collections
            .iter()
            .filter(|c| path.starts_with(c))
            .max_by_key(|c| c.components().count());
        if let Some(collection) = owner
            && exists(path)
        {
            if active == Some(i) {
                active_kept = Some(kept.len());
            }
            kept.push(TabFile {
                path: path.clone(),
                collection: collection.clone(),
            });
        }
    }
    let active = active_kept.or_else(|| {
        kept.len()
            .checked_sub(1)
            .map(|last| active.unwrap_or(0).min(last))
    });
    (kept, active)
}
