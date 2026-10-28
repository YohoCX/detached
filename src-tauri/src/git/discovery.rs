use crate::types::RepoInfo;

// Scaffold fixture; agent A implements the walk from PLAN.md "Discovery".
pub fn discover(folders: &[String], excluded: &[String]) -> Vec<RepoInfo> {
    let _ = excluded;
    folders
        .iter()
        .flat_map(|f| {
            ["api", "web"].map(|n| RepoInfo {
                path: format!("{f}/{n}"),
                name: n.into(),
                folder: f.clone(),
            })
        })
        .collect()
}
