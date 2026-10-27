use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use crate::Request;

/// Extension of request files.
pub const REQUEST_EXT: &str = ".req.toml";

/// A request together with the file it lives in.
#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub request: Request,
}

/// Finds every `*.req.toml` under `dir`, sorted by path. Unreadable files are
/// returned as errors next to the entries so one bad file does not hide the rest.
pub fn load(dir: &Path) -> (Vec<Entry>, Vec<anyhow::Error>) {
    let mut paths = Vec::new();
    collect(dir, &mut paths);
    paths.sort();

    let mut entries = Vec::new();
    let mut errors = Vec::new();
    for path in paths {
        match Request::load(&path) {
            Ok(request) => entries.push(Entry { path, request }),
            Err(err) => errors.push(err),
        }
    }
    (entries, errors)
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };
    for item in read_dir.flatten() {
        let path = item.path();
        let name = item.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "target" || name == "node_modules" {
            continue;
        }
        if path.is_dir() {
            collect(&path, out);
        } else if name.ends_with(REQUEST_EXT) {
            out.push(path);
        }
    }
}

/// Creates a new request file in `dir` with a free `request-N.req.toml` name.
pub fn create(dir: &Path) -> Result<Entry> {
    let n = (1..)
        .find(|n| !dir.join(format!("request-{n}{REQUEST_EXT}")).exists())
        .expect("unbounded range");
    let path = dir.join(format!("request-{n}{REQUEST_EXT}"));
    let request = Request::new(format!("Request {n}"));
    request.save(&path)?;
    Ok(Entry { path, request })
}

/// Renames the request and moves its file to a name derived from the new one,
/// e.g. `Create user` → `create-user.req.toml`, next to the old file.
pub fn rename(entry: &Entry, name: &str) -> Result<Entry> {
    let mut request = entry.request.clone();
    request.name = name.trim().to_string();

    let dir = entry.path.parent().unwrap_or(Path::new("."));
    let stem = slug(&request.name);
    let path = (1..)
        .map(|n| match n {
            1 => dir.join(format!("{stem}{REQUEST_EXT}")),
            n => dir.join(format!("{stem}-{n}{REQUEST_EXT}")),
        })
        .find(|path| *path == entry.path || !path.exists())
        .expect("unbounded range");

    request.save(&path)?;
    if path != entry.path {
        std::fs::remove_file(&entry.path)
            .with_context(|| format!("removing {}", entry.path.display()))?;
    }
    Ok(Entry { path, request })
}

/// Deletes the request's file.
pub fn delete(entry: &Entry) -> Result<()> {
    std::fs::remove_file(&entry.path).with_context(|| format!("removing {}", entry.path.display()))
}

/// File-name friendly form of `name`: lowercase letters and digits joined by `-`.
fn slug(name: &str) -> String {
    let mut slug = String::new();
    for ch in name.chars() {
        if ch.is_alphanumeric() {
            slug.extend(ch.to_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        "request".into()
    } else {
        slug.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug("Create user"), "create-user");
        assert_eq!(slug("  GET /users/{id}  "), "get-users-id");
        assert_eq!(slug("Получить задачи"), "получить-задачи");
        assert_eq!(slug("???"), "request");
    }

    #[test]
    fn rename_moves_file_and_avoids_collisions() {
        let dir = std::env::temp_dir().join(format!("reqo-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let a = create(&dir).unwrap();
        let b = create(&dir).unwrap();
        let a = rename(&a, "Users").unwrap();
        assert_eq!(a.path, dir.join("users.req.toml"));
        assert_eq!(Request::load(&a.path).unwrap().name, "Users");

        let b_old = b.path.clone();
        let b = rename(&b, "users").unwrap();
        assert_eq!(b.path, dir.join("users-2.req.toml"));
        assert!(!b_old.exists());

        // Renaming to the same slug keeps the file.
        let a = rename(&a, "USERS").unwrap();
        assert_eq!(a.path, dir.join("users.req.toml"));

        delete(&a).unwrap();
        delete(&b).unwrap();
        assert!(collection_is_empty(&dir));
        std::fs::remove_dir(&dir).unwrap();
    }

    fn collection_is_empty(dir: &Path) -> bool {
        load(dir).0.is_empty()
    }
}
