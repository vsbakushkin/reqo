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
    for (path, is_dir) in children(dir) {
        if is_dir {
            collect(&path, out);
        } else if path.to_string_lossy().ends_with(REQUEST_EXT) {
            out.push(path);
        }
    }
}

/// Every folder under `dir`, sorted by path, including ones without requests.
pub fn folders(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for (path, is_dir) in children(dir) {
            if is_dir {
                out.push(path.clone());
                walk(&path, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.sort();
    out
}

/// Entries of `dir` with whether each is a directory, skipping hidden and build folders.
fn children(dir: &Path) -> Vec<(PathBuf, bool)> {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    read_dir
        .flatten()
        .filter(|item| {
            let name = item.file_name();
            let name = name.to_string_lossy();
            !(name.starts_with('.') || name == "target" || name == "node_modules")
        })
        .map(|item| {
            let path = item.path();
            let is_dir = path.is_dir();
            (path, is_dir)
        })
        .collect()
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
    let path = free_path(dir, &slug(&request.name), REQUEST_EXT, Some(&entry.path));

    request.save(&path)?;
    if path != entry.path {
        std::fs::remove_file(&entry.path)
            .with_context(|| format!("removing {}", entry.path.display()))?;
    }
    Ok(Entry { path, request })
}

/// Creates a `New Folder` (or `New Folder-N`) folder in `parent`.
pub fn create_folder(parent: &Path) -> Result<PathBuf> {
    let path = free_path(parent, "New Folder", "", None);
    std::fs::create_dir(&path).with_context(|| format!("creating {}", path.display()))?;
    Ok(path)
}

/// Renames the folder, keeping it in the same parent. Folder names are shown
/// in the app as they are, so only characters a file name cannot hold change.
pub fn rename_folder(path: &Path, name: &str) -> Result<PathBuf> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let new_path = free_path(parent, &folder_name(name), "", Some(path));
    if new_path != path {
        std::fs::rename(path, &new_path).with_context(|| format!("renaming {}", path.display()))?;
    }
    Ok(new_path)
}

/// Moves the request's file into `dir`, keeping its file name unless that is
/// taken there.
pub fn move_entry(entry: &Entry, dir: &Path) -> Result<Entry> {
    if entry.path.parent() == Some(dir) {
        return Ok(entry.clone());
    }
    let file_name = entry.path.file_name().unwrap_or_default().to_string_lossy();
    let stem = file_name.strip_suffix(REQUEST_EXT).unwrap_or(&file_name);
    let path = free_path(dir, stem, REQUEST_EXT, None);
    move_path(&entry.path, &path)?;
    Ok(Entry {
        path,
        request: entry.request.clone(),
    })
}

/// Moves the folder into `dir`, keeping its name unless that is taken there.
/// A folder cannot go into itself or one of its subfolders.
pub fn move_folder(path: &Path, dir: &Path) -> Result<PathBuf> {
    if dir.starts_with(path) {
        anyhow::bail!("a folder cannot be moved into itself");
    }
    if path.parent() == Some(dir) {
        return Ok(path.to_path_buf());
    }
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let new_path = free_path(dir, &name, "", None);
    move_path(path, &new_path)?;
    Ok(new_path)
}

/// Renames, or copies and deletes when `from` and `to` are on different filesystems.
fn move_path(from: &Path, to: &Path) -> Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_path(from, to)
        .with_context(|| format!("moving {} to {}", from.display(), to.display()))?;
    if from.is_dir() {
        std::fs::remove_dir_all(from)
    } else {
        std::fs::remove_file(from)
    }
    .with_context(|| format!("removing {}", from.display()))
}

fn copy_path(from: &Path, to: &Path) -> std::io::Result<()> {
    if !from.is_dir() {
        return std::fs::copy(from, to).map(|_| ());
    }
    std::fs::create_dir(to)?;
    for item in std::fs::read_dir(from)? {
        let item = item?;
        copy_path(&item.path(), &to.join(item.file_name()))?;
    }
    Ok(())
}

/// Deletes the folder with everything in it.
pub fn delete_folder(path: &Path) -> Result<()> {
    std::fs::remove_dir_all(path).with_context(|| format!("removing {}", path.display()))
}

/// `dir/stem+ext`, or `dir/stem-N+ext` when taken. `current` counts as free,
/// so renaming something to its own name keeps it in place.
pub(crate) fn free_path(dir: &Path, stem: &str, ext: &str, current: Option<&Path>) -> PathBuf {
    (1..)
        .map(|n| match n {
            1 => dir.join(format!("{stem}{ext}")),
            n => dir.join(format!("{stem}-{n}{ext}")),
        })
        .find(|path| Some(path.as_path()) == current || !path.exists())
        .expect("unbounded range")
}

/// Deletes the request's file.
pub fn delete(entry: &Entry) -> Result<()> {
    std::fs::remove_file(&entry.path).with_context(|| format!("removing {}", entry.path.display()))
}

/// `name` usable as a folder name: no path separators, not hidden.
fn folder_name(name: &str) -> String {
    let name: String = name
        .trim()
        .chars()
        .map(|ch| {
            if matches!(ch, '/' | '\\' | '\0') {
                '-'
            } else {
                ch
            }
        })
        .collect();
    let name = name.trim_start_matches('.');
    if name.is_empty() {
        "Folder".into()
    } else {
        name.into()
    }
}

/// File-name friendly form of `name`: lowercase letters and digits joined by `-`.
pub(crate) fn slug(name: &str) -> String {
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

    #[test]
    fn folders_are_listed_renamed_and_deleted() {
        let dir = std::env::temp_dir().join(format!("reqo-test-folders-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".git")).unwrap();

        let a = create_folder(&dir).unwrap();
        let b = create_folder(&dir).unwrap();
        assert_eq!(b, dir.join("New Folder-2"));
        let a = rename_folder(&a, "Users API").unwrap();
        assert_eq!(a, dir.join("Users API"));
        assert_eq!(folder_name(" ../a/b "), "-a-b");
        let nested = create_folder(&a).unwrap();
        create(&nested).unwrap();
        assert_eq!(folders(&dir), vec![b.clone(), a.clone(), nested]);
        assert_eq!(load(&dir).0.len(), 1);

        delete_folder(&a).unwrap();
        assert_eq!(folders(&dir), vec![b]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn moves_requests_and_folders() {
        let dir = std::env::temp_dir().join(format!("reqo-test-move-{}", std::process::id()));
        let (a, b) = (dir.join("a"), dir.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(b.join("nested")).unwrap();

        let first = create(&a).unwrap();
        let taken = create(&b).unwrap();
        assert_eq!(taken.path, b.join("request-1.req.toml"));
        // The name is taken in `b`, so the moved file gets the next free one.
        let moved = move_entry(&first, &b).unwrap();
        assert_eq!(moved.path, b.join("request-1-2.req.toml"));
        assert!(!first.path.exists());
        assert_eq!(move_entry(&moved, &b).unwrap().path, moved.path);

        assert!(move_folder(&b, &b.join("nested")).is_err());
        assert!(move_folder(&b, &b).is_err());
        let b_moved = move_folder(&b, &a).unwrap();
        assert_eq!(b_moved, a.join("b"));
        assert_eq!(load(&dir).0.len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn collection_is_empty(dir: &Path) -> bool {
        load(dir).0.is_empty()
    }
}
