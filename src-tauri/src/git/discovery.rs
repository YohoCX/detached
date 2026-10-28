use std::collections::BTreeMap;
use std::path::Path;

use crate::types::RepoInfo;

const DEPTH: u32 = 3;

/// Repos in and up to 3 levels below each folder, sorted by path; the first folder wins a duplicate.
pub fn discover(folders: &[String], excluded: &[String]) -> Vec<RepoInfo> {
    let mut repos = BTreeMap::new();
    for f in folders {
        let f = f.trim_end_matches('/');
        walk(Path::new(f), 0, f, excluded, &mut repos);
    }
    repos.into_values().collect()
}

fn walk(
    dir: &Path,
    depth: u32,
    folder: &str,
    excluded: &[String],
    out: &mut BTreeMap<String, RepoInfo>,
) {
    let path = dir.to_string_lossy().into_owned();
    if excluded.contains(&path) {
        return;
    }
    if dir.join(".git").exists() {
        let name = dir
            .file_name()
            .map_or(path.clone(), |n| n.to_string_lossy().into());
        out.entry(path.clone()).or_insert(RepoInfo {
            path,
            name,
            folder: folder.into(),
        });
        return;
    }
    if depth == DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with('.')
            && name != "node_modules"
            && e.file_type().is_ok_and(|t| t.is_dir())
        {
            walk(&e.path(), depth + 1, folder, excluded, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn walks_three_levels_and_skips_what_it_should() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let repo = |p: &str| fs::create_dir_all(root.join(p).join(".git")).unwrap();
        repo("a");
        repo("a/nested"); // inside a repo: not listed
        repo("x/y/z");
        repo("x/y/z2/deep"); // 4 levels below the root, 3 below `x`
        repo("node_modules/pkg");
        repo(".hidden/r");
        repo("excluded");
        fs::create_dir_all(root.join("x/wt")).unwrap();
        fs::write(root.join("x/wt/.git"), "gitdir: /elsewhere\n").unwrap();

        let root_s = root.to_str().unwrap().to_string();
        let x = format!("{root_s}/x");
        let excluded = vec![format!("{root_s}/excluded")];
        let found: Vec<(String, String)> = discover(&[root_s.clone(), format!("{x}/")], &excluded)
            .into_iter()
            .map(|r| (r.path.strip_prefix(&root_s).unwrap().into(), r.folder))
            .collect();
        let want = [
            ("/a", &root_s),
            ("/x/wt", &root_s),
            ("/x/y/z", &root_s),
            ("/x/y/z2/deep", &x),
        ];
        assert_eq!(found, want.map(|(p, f)| (p.to_string(), f.clone())));
        assert!(
            discover(std::slice::from_ref(&root_s), &excluded)
                .iter()
                .all(|r| !r.path.ends_with("deep"))
        );
    }

    #[test]
    fn a_folder_that_is_a_repo_counts() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join(".git")).unwrap();
        let f = tmp.path().to_str().unwrap().to_string();
        let repos = discover(std::slice::from_ref(&f), &[]);
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].path, f);
    }
}
