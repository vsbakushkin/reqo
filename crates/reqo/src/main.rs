mod app;
mod env_editor;
mod settings;
mod theme;

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use gpui_kit::component::Root;
use gpui_kit::*;

use reqo_core::workspace;

use crate::app::{
    DEFAULT_WORKSPACE, DeleteRequest, KEY_CONTEXT, NewRequest, OpenSettings, Reload, RenameRequest,
    ReqoApp, SaveRequest, SendRequest, ToggleTheme,
};

actions!(reqo, [Quit]);

// Icons outside the default component set that the app uses.
gpui_kit::assets::icon_assets!(
    ExtraIcons,
    [
        Send,
        RefreshCw,
        Pencil,
        Trash,
        Layers,
        LibraryBig,
        FolderPlus,
        FolderInput,
        FolderSymlink,
        Unlink
    ]
);

/// The default component icons plus [`ExtraIcons`].
struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

/// Where workspaces live: `$XDG_DATA_HOME/reqo`, or `~/.local/share/reqo`.
fn data_root() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("share"))
        })
        .unwrap_or_else(|| PathBuf::from("."))
        .join("reqo")
}

fn main() {
    let root = data_root();
    let mut workspace = workspace::open_last(&root, DEFAULT_WORKSPACE).unwrap_or_else(|err| {
        eprintln!("reqo: {err:#}");
        std::process::exit(1);
    });
    // A folder given on the command line joins the workspace as a collection.
    let reveal = std::env::args_os().nth(1).and_then(|dir| {
        match workspace.add_collection(Path::new(&dir)) {
            Ok(ix) => Some(workspace.collection_dir(&workspace.collections[ix])),
            Err(err) => {
                eprintln!("reqo: {err:#}");
                None
            }
        }
    });
    if let Err(err) = workspace::remember(&root, &workspace) {
        eprintln!("reqo: {err:#}");
    }

    gpui_kit::application()
        .with_assets(AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            theme::init(cx);

            cx.bind_keys([
                KeyBinding::new("ctrl-enter", SendRequest, None),
                KeyBinding::new("ctrl-s", SaveRequest, None),
                KeyBinding::new("ctrl-n", NewRequest, None),
                KeyBinding::new("ctrl-r", Reload, None),
                KeyBinding::new("ctrl-shift-t", ToggleTheme, None),
                KeyBinding::new("ctrl-,", OpenSettings, None),
                KeyBinding::new("f2", RenameRequest, None),
                // Only outside inputs, where Delete edits text.
                KeyBinding::new("delete", DeleteRequest, Some(KEY_CONTEXT)),
                KeyBinding::new("ctrl-q", Quit, None),
            ]);
            cx.on_action(|_: &Quit, cx| cx.quit());

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1200.), px(800.)), cx)),
                app_id: Some("reqo".into()),
                ..Default::default()
            };
            cx.open_window(options, |window, cx| {
                let view = cx.new(|cx| {
                    ReqoApp::new(root.clone(), workspace.clone(), reveal.clone(), window, cx)
                });
                // Root hosts dialogs, notifications and popovers above the view.
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open window");
            cx.activate(true);
        });
}
