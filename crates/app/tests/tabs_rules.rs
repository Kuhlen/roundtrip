use std::path::{Path, PathBuf};

use app::modules::workspace::tabs::tabs_rules::{
    TabFile, next_active, rebase_path, reorder_target, restore_tabs, untitled_title,
};

#[test]
fn next_active_prefers_the_right_neighbour() {
    assert_eq!(next_active(2, 0), Some(0), "closed first of three");
    assert_eq!(next_active(2, 1), Some(1), "closed middle");
    assert_eq!(
        next_active(2, 2),
        Some(1),
        "closed last falls back to the new last"
    );
    assert_eq!(next_active(0, 0), None, "closed the only tab");
}

#[test]
fn reorder_stays_inside_the_pin_group() {
    let pinned = [true, true, false, false, false];
    assert_eq!(
        reorder_target(&pinned, 3, 0),
        2,
        "unpinned cannot pass pinned"
    );
    assert_eq!(
        reorder_target(&pinned, 0, 4),
        1,
        "pinned cannot pass unpinned"
    );
    assert_eq!(reorder_target(&pinned, 2, 4), 4);
    assert_eq!(reorder_target(&[], 0, 3), 0);
}

#[test]
fn untitled_titles_fill_gaps() {
    assert_eq!(untitled_title(&[]), "Untitled");
    assert_eq!(untitled_title(&["Untitled"]), "Untitled 2");
    assert_eq!(untitled_title(&["Untitled", "Untitled 3"]), "Untitled 2");
    assert_eq!(untitled_title(&["Untitled 2"]), "Untitled");
}

#[test]
fn rebase_moves_paths_under_the_old_one_only() {
    let old = Path::new("/c/users");
    let new = Path::new("/c/people");
    assert_eq!(
        rebase_path(Path::new("/c/users"), old, new),
        Some(PathBuf::from("/c/people"))
    );
    assert_eq!(
        rebase_path(Path::new("/c/users/list.yaml"), old, new),
        Some(PathBuf::from("/c/people/list.yaml"))
    );
    assert_eq!(
        rebase_path(Path::new("/c/users-old/x.yaml"), old, new),
        None
    );
}

#[test]
fn restore_keeps_existing_files_of_open_collections() {
    let paths = [
        PathBuf::from("/c/a.yaml"),
        PathBuf::from("/gone/b.yaml"),
        PathBuf::from("/c/missing.yaml"),
        PathBuf::from("/d/sub/c.yaml"),
    ];
    let collections = [PathBuf::from("/c"), PathBuf::from("/d")];
    let exists = |p: &Path| p != Path::new("/c/missing.yaml");
    let (tabs, active) = restore_tabs(&paths, Some(3), &collections, exists);
    assert_eq!(
        tabs,
        vec![
            TabFile {
                path: "/c/a.yaml".into(),
                collection: "/c".into()
            },
            TabFile {
                path: "/d/sub/c.yaml".into(),
                collection: "/d".into()
            },
        ]
    );
    assert_eq!(active, Some(1), "active index follows its tab");
    let (_, active) = restore_tabs(&paths, Some(2), &collections, exists);
    assert_eq!(
        active,
        Some(1),
        "skipped active tab clamps to the last kept"
    );
    let (none, active) = restore_tabs(&paths[1..2], Some(0), &collections, exists);
    assert!(none.is_empty());
    assert_eq!(active, None);
}

#[test]
fn restore_picks_the_deepest_collection() {
    let collections = [PathBuf::from("/c"), PathBuf::from("/c/nested")];
    let (tabs, _) = restore_tabs(
        &[PathBuf::from("/c/nested/x.yaml")],
        None,
        &collections,
        |_| true,
    );
    assert_eq!(tabs[0].collection, PathBuf::from("/c/nested"));
}
