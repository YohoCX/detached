//! Quick open: cached file lists per (repo, rev), ranked with nucleo.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::env::Env;
use crate::error::Result;
use crate::git::files;
use crate::types::FileMatch;

type Key = (String, Option<String>);

/// Cleared per repo by `invalidate` on every status refresh (after each operation and on focus).
static CACHE: LazyLock<Mutex<HashMap<Key, Arc<Vec<String>>>>> = LazyLock::new(Default::default);

pub fn paths(env: &Env, repo: &str, rev: Option<&str>) -> Result<Arc<Vec<String>>> {
    let key = (repo.to_string(), rev.map(String::from));
    if let Some(p) = CACHE.lock().unwrap().get(&key) {
        return Ok(p.clone());
    }
    let p = Arc::new(files::paths(env, Path::new(repo), rev)?);
    CACHE.lock().unwrap().insert(key, p.clone());
    Ok(p)
}

pub fn invalidate(repo: &str) {
    CACHE.lock().unwrap().retain(|(r, _), _| r != repo);
}

/// The best `limit` matches across `lists` of `(repo, paths)`; ties go to the shorter path.
pub fn rank(lists: &[(&str, Arc<Vec<String>>)], query: &str, limit: usize) -> Vec<FileMatch> {
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let mut buf = vec![];
    let mut hits: Vec<(u32, &str, &str)> = vec![];
    for (repo, paths) in lists {
        for p in paths.iter() {
            if let Some(score) = pattern.score(Utf32Str::new(p, &mut buf), &mut matcher) {
                hits.push((score, repo, p));
            }
        }
    }
    let order = |a: &(u32, &str, &str), b: &(u32, &str, &str)| {
        (b.0.cmp(&a.0))
            .then(a.2.len().cmp(&b.2.len()))
            .then(a.2.cmp(b.2))
    };
    if hits.len() > limit && limit > 0 {
        hits.select_nth_unstable_by(limit - 1, order);
    }
    hits.truncate(limit);
    hits.sort_unstable_by(order);
    hits.into_iter()
        .map(|(score, repo, path)| {
            let mut indices = vec![];
            pattern.indices(Utf32Str::new(path, &mut buf), &mut matcher, &mut indices);
            indices.sort_unstable();
            indices.dedup();
            FileMatch {
                repo: repo.into(),
                path: path.into(),
                score,
                indices,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;
    use std::time::{Duration, Instant};

    fn list(paths: &[&str]) -> Arc<Vec<String>> {
        Arc::new(paths.iter().map(|p| p.to_string()).collect())
    }

    #[test]
    fn ranks_and_highlights_across_repos() {
        let a = list(&[
            "src/main.rs",
            "src/lib/domain.rs",
            "README.md",
            "docs/manual/index.md",
        ]);
        let b = list(&["web/src/Main.svelte", "Makefile"]);
        let m = rank(&[("a", a), ("b", b)], "main", 10);
        let found: Vec<_> = m
            .iter()
            .map(|m| (m.repo.as_str(), m.path.as_str()))
            .collect();
        assert_eq!(
            found[..2],
            [("a", "src/main.rs"), ("b", "web/src/Main.svelte")]
        );
        assert!(!found.contains(&("a", "README.md")));
        assert_eq!(m[0].indices, [4, 5, 6, 7]);

        let m = rank(
            &[(
                "a",
                list(&["src/components/FileTree.svelte", "src/lib/fuzzy.ts"]),
            )],
            "ftree",
            1,
        );
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].path, "src/components/FileTree.svelte");
        let chars: String = m[0]
            .indices
            .iter()
            .map(|&i| m[0].path.chars().nth(i as usize).unwrap())
            .collect();
        assert_eq!(chars.to_lowercase(), "ftree");
    }

    #[test]
    fn lists_are_cached_per_repo_and_rev() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        sb.commit(&work, ".gitignore", "*.log\n", "ignore");
        std::fs::write(work.join("x.log"), "").unwrap();
        std::fs::write(work.join("new.txt"), "").unwrap();
        let (env, repo) = (sb.state.env(), work.to_str().unwrap());
        assert_eq!(
            *paths(&env, repo, None).unwrap(),
            [".gitignore", "f", "new.txt"]
        );
        assert_eq!(*paths(&env, repo, Some("HEAD~1")).unwrap(), ["f"]);
        std::fs::write(work.join("later.txt"), "").unwrap();
        let has_later = || {
            paths(&env, repo, None)
                .unwrap()
                .contains(&"later.txt".to_string())
        };
        assert!(!has_later());
        invalidate(repo);
        assert!(has_later());
    }

    #[test]
    fn fifty_thousand_files_rank_in_under_100_ms() {
        let dirs = [
            "src",
            "app/components",
            "lib/util",
            "tests/integration",
            "packages/core/src",
        ];
        let names = [
            "index", "button", "parser", "config", "router", "store", "view", "model",
        ];
        let paths: Vec<String> = (0..50_000)
            .map(|i| format!("{}/m{}/{}{}.ts", dirs[i % 5], i / 40, names[i % 8], i % 97))
            .collect();
        let lists = [("repo", Arc::new(paths))];
        rank(&lists, "warm", 50);
        let start = Instant::now();
        let m = rank(&lists, "compbutton", 50);
        let took = start.elapsed();
        eprintln!("quick open over 50,000 files: {took:?}");
        assert_eq!(m.len(), 50);
        assert!(m[0].path.starts_with("app/components/"));
        assert!(took < Duration::from_millis(100), "{took:?}");
    }
}
