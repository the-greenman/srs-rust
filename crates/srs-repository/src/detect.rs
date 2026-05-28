use crate::error::RepositoryError;
use std::path::{Path, PathBuf};

/// Walk up from `start` until a directory containing `.srs/` is found.
/// Returns the repository root.
pub fn find_repo_root(start: &Path) -> Result<PathBuf, RepositoryError> {
    let mut current = start.to_path_buf();

    loop {
        let srs_dir = current.join(".srs");
        if srs_dir.is_dir() {
            return Ok(current);
        }

        if !current.pop() {
            return Err(RepositoryError::NotFound {
                path: start.to_path_buf(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn find_repo_root_from_nested_path() {
        let temp = TempDir::new().unwrap();
        let repo_root = temp.path().join("repo");
        let nested = repo_root.join("records").join("notes");

        fs::create_dir_all(&nested).unwrap();
        fs::create_dir(repo_root.join(".srs")).unwrap();

        let result = find_repo_root(&nested).unwrap();
        assert_eq!(result, repo_root);
    }

    #[test]
    fn find_repo_root_returns_not_found() {
        let temp = TempDir::new().unwrap();
        let start = temp.path().join("nowhere");

        fs::create_dir_all(&start).unwrap();

        let result = find_repo_root(&start);
        assert_eq!(
            result,
            Err(RepositoryError::NotFound {
                path: start.clone()
            })
        );
    }
}
