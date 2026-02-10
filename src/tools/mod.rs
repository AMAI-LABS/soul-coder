pub mod bash;
pub mod edit;
pub mod find;
pub mod grep;
pub mod ls;
pub mod read;
pub mod write;

/// Resolve a path relative to the working directory.
/// Absolute paths are returned as-is; relative paths are joined with cwd.
pub(crate) fn resolve_path(cwd: &str, path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("{}/{}", cwd.trim_end_matches('/'), path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_path_unchanged() {
        assert_eq!(resolve_path("/project", "/abs/file.txt"), "/abs/file.txt");
    }

    #[test]
    fn relative_path_joined() {
        assert_eq!(resolve_path("/project", "src/main.rs"), "/project/src/main.rs");
    }

    #[test]
    fn cwd_trailing_slash_stripped() {
        assert_eq!(resolve_path("/project/", "file.txt"), "/project/file.txt");
    }
}
