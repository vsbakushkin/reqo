mod app;
mod theme;

use std::borrow::Cow;
use std::path::PathBuf;

use gpui_kit::component::Root;
use gpui_kit::*;

use crate::app::{
    DeleteRequest, KEY_CONTEXT, NewRequest, Reload, RenameRequest, ReqoApp, SaveRequest,
    SendRequest, ToggleTheme,
};

actions!(reqo, [Quit]);

// Icons outside the default component set that the app uses.
gpui_kit::assets::icon_assets!(
    ExtraIcons,
    [Send, RefreshCw, Sun, Moon, Palette, Pencil, Trash]
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

fn main() {
    // The collection is the directory given on the command line, or the current one.
    let dir = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("current directory"));
    let dir = dir.canonicalize().unwrap_or(dir);

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
                let view = cx.new(|cx| ReqoApp::new(dir, window, cx));
                // Root hosts dialogs, notifications and popovers above the view.
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open window");
            cx.activate(true);
        });
}
