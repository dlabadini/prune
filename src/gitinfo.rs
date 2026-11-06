//! Recent history via gix, used to mark findings in files that just changed.

use gix::bstr::ByteSlice;
use std::collections::HashSet;
use std::path::Path;

/// Relative paths touched in the last `max_commits` commits.
///
/// Returns an empty set when the directory is not a git repository.
pub fn recent_paths(root: &Path, max_commits: usize) -> HashSet<String> {
    let mut paths = HashSet::new();
    let Ok(repo) = gix::discover(root) else {
        return paths;
    };
    let Ok(head) = repo.head_id() else {
        return paths;
    };
    let Ok(walk) = repo.rev_walk([head.detach()]).all() else {
        return paths;
    };

    for info in walk.take(max_commits) {
        let Ok(info) = info else {
            continue;
        };
        let Ok(commit) = info.object() else {
            continue;
        };
        let Ok(tree) = commit.tree() else {
            continue;
        };
        let parent = info.parent_ids().next();
        let changes = if let Some(parent_id) = parent {
            let Ok(parent_commit) = repo.find_commit(parent_id.detach()) else {
                continue;
            };
            let Ok(parent_tree) = parent_commit.tree() else {
                continue;
            };
            repo.diff_tree_to_tree(Some(&parent_tree), Some(&tree), None)
        } else {
            repo.diff_tree_to_tree(None, Some(&tree), None)
        };
        let Ok(changes) = changes else {
            continue;
        };
        for change in changes {
            collect_locations(&change, &mut paths);
        }
    }

    paths
}

fn collect_locations(
    change: &gix::object::tree::diff::ChangeDetached,
    paths: &mut HashSet<String>,
) {
    match change {
        gix::object::tree::diff::ChangeDetached::Addition { location, .. }
        | gix::object::tree::diff::ChangeDetached::Deletion { location, .. }
        | gix::object::tree::diff::ChangeDetached::Modification { location, .. } => {
            push_path(location, paths);
        }
        gix::object::tree::diff::ChangeDetached::Rewrite {
            location,
            source_location,
            ..
        } => {
            push_path(location, paths);
            push_path(source_location, paths);
        }
    }
}

fn push_path(location: &gix::bstr::BString, paths: &mut HashSet<String>) {
    if let Ok(text) = location.to_str() {
        if !text.is_empty() {
            paths.insert(text.replace('\\', "/"));
        }
    }
}

/// Note findings whose files show up in recent history.
pub fn annotate_findings(root: &Path, findings: &mut [crate::patterns::Finding]) {
    let recent = recent_paths(root, 30);
    if recent.is_empty() {
        return;
    }
    for finding in findings {
        let touched = finding.locations.iter().any(|location| {
            let normalized = location.file.replace('\\', "/");
            recent.contains(&normalized) || recent.contains(normalized.trim_start_matches("./"))
        });
        if touched
            && !finding
                .related_concepts
                .iter()
                .any(|item| item == "git:recent")
        {
            finding.related_concepts.push("git:recent".to_string());
            finding
                .description
                .push_str("\n\nTouched in recent git history.");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_history_of_this_repo() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let paths = recent_paths(root, 5);
        assert!(!paths.is_empty());
    }
}
