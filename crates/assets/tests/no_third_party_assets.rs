//! `docs/legal.md` rule 2: no files from the game we are compatible with in
//! the repository. Fails if a client archive, an installation folder or an
//! original asset path appears anywhere in the source tree (T1-14 guard).

use std::path::{Path, PathBuf};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if name == "target" || name == ".git" {
            continue;
        }
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

#[test]
fn repository_holds_no_original_game_files() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    walk(&root, &mut files);
    let bad: Vec<String> = files
        .iter()
        .map(|p| {
            p.strip_prefix(&root)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/")
        })
        .filter(|p| {
            p.ends_with(".jar")
                || p.contains(".minecraft/")
                || p.contains("assets/minecraft/")
                || p.contains("/versions/") && p.ends_with(".json")
        })
        .collect();
    assert!(
        bad.is_empty(),
        "original game files must never be committed: {bad:?}"
    );
}
