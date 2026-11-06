//! Local project layout: generated data under `.prune/`, committed config at the root.

use std::io;
use std::path::{Path, PathBuf};

pub const DATA_DIR_NAME: &str = ".prune";
pub const GITIGNORE_ENTRY: &str = ".prune/";

pub fn data_dir(project_root: &Path) -> PathBuf {
    project_root.join(DATA_DIR_NAME)
}

pub fn models_dir(project_root: &Path) -> PathBuf {
    data_dir(project_root).join("models")
}

/// Create `.prune/` and a nested gitignore so `git add .` does not pick up the index.
pub fn ensure_data_dir(project_root: &Path) -> io::Result<PathBuf> {
    let dir = data_dir(project_root);
    std::fs::create_dir_all(&dir)?;
    let ignore = dir.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(ignore, "*\n")?;
    }
    Ok(dir)
}

/// Append `.prune/` to the repo `.gitignore` when it is missing.
/// Returns true when a line was added.
pub fn ensure_gitignore(project_root: &Path) -> io::Result<bool> {
    let path = project_root.join(".gitignore");
    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        if gitignore_covers_prune(&content) {
            return Ok(false);
        }
        let mut next = content;
        if !next.is_empty() && !next.ends_with('\n') {
            next.push('\n');
        }
        if !next.is_empty() {
            next.push('\n');
        }
        next.push_str("# Prune local index\n");
        next.push_str(GITIGNORE_ENTRY);
        next.push('\n');
        std::fs::write(&path, next)?;
        Ok(true)
    } else {
        std::fs::write(path, "# Prune local index\n.prune/\n")?;
        Ok(true)
    }
}

fn gitignore_covers_prune(content: &str) -> bool {
    content.lines().any(|line| {
        let trimmed = line.trim();
        matches!(
            trimmed,
            ".prune/" | ".prune" | "/.prune/" | "/.prune" | "**/.prune/" | "**/.prune"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn data_dir_contains_its_own_gitignore() {
        let dir = tempdir().unwrap();
        let prune = ensure_data_dir(dir.path()).unwrap();
        assert_eq!(prune, dir.path().join(".prune"));
        assert_eq!(std::fs::read_to_string(prune.join(".gitignore")).unwrap(), "*\n");
        ensure_data_dir(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(prune.join(".gitignore")).unwrap(), "*\n");
    }

    #[test]
    fn gitignore_is_created_and_not_duplicated() {
        let dir = tempdir().unwrap();
        assert!(ensure_gitignore(dir.path()).unwrap());
        assert!(std::fs::read_to_string(dir.path().join(".gitignore"))
            .unwrap()
            .contains(".prune/"));
        assert!(!ensure_gitignore(dir.path()).unwrap());
        let content = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert_eq!(content.matches(".prune/").count(), 1);
    }

    #[test]
    fn gitignore_appends_to_existing_file() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "node_modules/\n").unwrap();
        assert!(ensure_gitignore(dir.path()).unwrap());
        let content = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains("node_modules/"));
        assert!(content.contains(".prune/"));
    }
}
