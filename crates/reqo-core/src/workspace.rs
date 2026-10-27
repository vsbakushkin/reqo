//! Workspaces: named sets of collections with shared environments.
//!
//! Everything lives under a data root (`~/.local/share/reqo` in the app):
//!
//! ```text
//! state.toml                    which workspace was open last
//! workspaces/<slug>/
//!     workspace.toml            name and the list of collections
//!     reqo.env.toml             environments shared by all collections
//!     collections/<slug>/       collections created in the app
//! ```
//!
//! A collection can also be any folder on disk, e.g. inside a project's
//! repository; the workspace then only remembers its path.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use crate::collection::{free_path, slug};

pub const WORKSPACE_FILE: &str = "workspace.toml";
const STATE_FILE: &str = "state.toml";
const WORKSPACES_DIR: &str = "workspaces";
const COLLECTIONS_DIR: &str = "collections";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectionRef {
    pub name: String,
    /// Relative to the workspace folder for collections created in the app,
    /// absolute for folders added from elsewhere.
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkspaceFile {
    name: String,
    #[serde(default, rename = "collection", skip_serializing_if = "Vec::is_empty")]
    collections: Vec<CollectionRef>,
}

#[derive(Debug, Clone)]
pub struct Workspace {
    pub dir: PathBuf,
    pub name: String,
    pub collections: Vec<CollectionRef>,
}

impl Workspace {
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join(WORKSPACE_FILE);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let file: WorkspaceFile =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        Ok(Self {
            dir: dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()),
            name: file.name,
            collections: file.collections,
        })
    }

    pub fn save(&self) -> Result<()> {
        let file = WorkspaceFile {
            name: self.name.clone(),
            collections: self.collections.clone(),
        };
        let path = self.dir.join(WORKSPACE_FILE);
        std::fs::write(&path, toml::to_string(&file)?)
            .with_context(|| format!("writing {}", path.display()))
    }

    /// Folder name of the workspace, used to remember it as the last one opened.
    pub fn id(&self) -> String {
        self.dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub fn collection_dir(&self, collection: &CollectionRef) -> PathBuf {
        self.dir.join(&collection.path)
    }

    /// Whether the collection was created in the app and is stored in the workspace.
    pub fn owns(&self, collection: &CollectionRef) -> bool {
        collection.path.is_relative()
    }

    /// Creates an empty collection inside the workspace folder.
    pub fn create_collection(&mut self, name: &str) -> Result<usize> {
        let parent = self.dir.join(COLLECTIONS_DIR);
        std::fs::create_dir_all(&parent)
            .with_context(|| format!("creating {}", parent.display()))?;
        let dir = free_path(&parent, &slug(name), "", None);
        std::fs::create_dir(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let path = dir.strip_prefix(&self.dir).unwrap_or(&dir).to_path_buf();
        self.collections.push(CollectionRef {
            name: name.trim().to_string(),
            path,
        });
        self.save()?;
        Ok(self.collections.len() - 1)
    }

    /// Adds an existing folder as a collection, or finds it if it is already there.
    pub fn add_collection(&mut self, dir: &Path) -> Result<usize> {
        if !dir.is_dir() {
            bail!("{} is not a folder", dir.display());
        }
        let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        if let Some(ix) = self
            .collections
            .iter()
            .position(|c| self.collection_dir(c) == dir)
        {
            return Ok(ix);
        }
        // Folders inside the workspace stay relative, so the workspace can be moved.
        let path = match dir.strip_prefix(&self.dir) {
            Ok(relative) => relative.to_path_buf(),
            Err(_) => dir.clone(),
        };
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| dir.display().to_string());
        self.collections.push(CollectionRef { name, path });
        self.save()?;
        Ok(self.collections.len() - 1)
    }

    /// Renames the collection. A collection the workspace owns also gets its
    /// folder renamed to match; folders added from elsewhere keep theirs.
    pub fn rename_collection(&mut self, ix: usize, name: &str) -> Result<()> {
        let collection = &self.collections[ix];
        if self.owns(collection) {
            let old = self.collection_dir(collection);
            let parent = old.parent().unwrap_or(&self.dir).to_path_buf();
            let new = free_path(&parent, &slug(name), "", Some(&old));
            if new != old {
                std::fs::rename(&old, &new)
                    .with_context(|| format!("renaming {}", old.display()))?;
                self.collections[ix].path =
                    new.strip_prefix(&self.dir).unwrap_or(&new).to_path_buf();
            }
        }
        self.collections[ix].name = name.trim().to_string();
        self.save()
    }

    /// Removes the collection from the workspace. Its files are deleted only
    /// when the workspace owns it; folders added from elsewhere are left alone.
    pub fn remove_collection(&mut self, ix: usize) -> Result<()> {
        let collection = self.collections.remove(ix);
        if self.owns(&collection) {
            let dir = self.collection_dir(&collection);
            if dir.exists() {
                std::fs::remove_dir_all(&dir)
                    .with_context(|| format!("removing {}", dir.display()))?;
            }
        }
        self.save()
    }
}

/// All workspaces under `root`, sorted by name.
pub fn list(root: &Path) -> Vec<Workspace> {
    let Ok(read_dir) = std::fs::read_dir(root.join(WORKSPACES_DIR)) else {
        return Vec::new();
    };
    let mut workspaces: Vec<Workspace> = read_dir
        .flatten()
        .filter_map(|item| Workspace::load(&item.path()).ok())
        .collect();
    workspaces.sort_by_key(|w| w.name.to_lowercase());
    workspaces
}

pub fn create(root: &Path, name: &str) -> Result<Workspace> {
    let parent = root.join(WORKSPACES_DIR);
    std::fs::create_dir_all(&parent).with_context(|| format!("creating {}", parent.display()))?;
    let dir = free_path(&parent, &slug(name), "", None);
    std::fs::create_dir(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let dir = dir.canonicalize().unwrap_or(dir);
    let workspace = Workspace {
        dir,
        name: name.trim().to_string(),
        collections: Vec::new(),
    };
    workspace.save()?;
    Ok(workspace)
}

/// Deletes the workspace folder with the collections it owns.
pub fn delete(workspace: &Workspace) -> Result<()> {
    std::fs::remove_dir_all(&workspace.dir)
        .with_context(|| format!("removing {}", workspace.dir.display()))
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workspace: Option<String>,
}

/// Opens the workspace used last, falling back to the first one and creating
/// `default_name` when there are none.
pub fn open_last(root: &Path, default_name: &str) -> Result<Workspace> {
    let last = std::fs::read_to_string(root.join(STATE_FILE))
        .ok()
        .and_then(|text| toml::from_str::<State>(&text).ok())
        .and_then(|state| state.workspace);
    let mut workspaces = list(root);
    if let Some(ix) = workspaces.iter().position(|w| Some(w.id()) == last) {
        return Ok(workspaces.swap_remove(ix));
    }
    match workspaces.into_iter().next() {
        Some(workspace) => Ok(workspace),
        None => create(root, default_name),
    }
}

/// Remembers `workspace` as the one to open next time.
pub fn remember(root: &Path, workspace: &Workspace) -> Result<()> {
    let state = State {
        workspace: Some(workspace.id()),
    };
    std::fs::create_dir_all(root).with_context(|| format!("creating {}", root.display()))?;
    let path = root.join(STATE_FILE);
    std::fs::write(&path, toml::to_string(&state)?)
        .with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspaces_and_collections() {
        let root = std::env::temp_dir().join(format!("reqo-test-ws-{}", std::process::id()));
        let external = root.join("elsewhere");
        std::fs::create_dir_all(&external).unwrap();

        // First start creates the default workspace.
        let mut ws = open_last(&root, "My Workspace").unwrap();
        assert_eq!(ws.name, "My Workspace");
        assert_eq!(ws.id(), "my-workspace");

        let owned = ws.create_collection("Tasks API").unwrap();
        assert_eq!(
            ws.collections[owned].path,
            Path::new("collections/tasks-api")
        );
        assert!(ws.collection_dir(&ws.collections[owned]).is_dir());

        let added = ws.add_collection(&external).unwrap();
        assert_eq!(ws.add_collection(&external).unwrap(), added);
        assert_eq!(ws.collections[added].name, "elsewhere");
        ws.rename_collection(added, "Billing").unwrap();

        let reloaded = Workspace::load(&ws.dir).unwrap();
        assert_eq!(reloaded.collections, ws.collections);

        // Removing deletes owned folders only.
        let owned_dir = ws.collection_dir(&ws.collections[owned]);
        ws.remove_collection(added).unwrap();
        assert!(external.is_dir());
        ws.remove_collection(owned).unwrap();
        assert!(!owned_dir.exists());

        let other = create(&root, "Other").unwrap();
        remember(&root, &other).unwrap();
        assert_eq!(open_last(&root, "x").unwrap().name, "Other");
        assert_eq!(
            list(&root)
                .iter()
                .map(|w| w.name.as_str())
                .collect::<Vec<_>>(),
            ["My Workspace", "Other"]
        );
        delete(&other).unwrap();
        assert_eq!(open_last(&root, "x").unwrap().name, "My Workspace");

        std::fs::remove_dir_all(&root).unwrap();
    }
}
