use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::env_editor::{EnvEditor, EnvEditorEvent, Scope};
use crate::settings::Settings;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Icon, IndexPath, Root, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariant, ButtonVariants as _},
    dialog::DialogButtonProps,
    h_flex,
    input::{Editor, EditorState, Input, InputEvent, InputState, Textarea, TextareaState},
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem},
    notification::Notification,
    resizable::{h_resizable, resizable_panel, v_resizable},
    select::{Select, SelectEvent, SelectState},
    tab::{Tab, TabBar},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use reqo_core::collection::{self, Entry};
use reqo_core::workspace::{self, Workspace};
use reqo_core::{Environments, METHODS, Request, Response, Variables};

actions!(
    reqo,
    [
        SendRequest,
        SaveRequest,
        NewRequest,
        Reload,
        ToggleTheme,
        RenameRequest,
        DeleteRequest,
        OpenSettings
    ]
);

/// Key context of the app view; bindings in it apply when no input has focus.
pub const KEY_CONTEXT: &str = "Reqo";

/// Workspace created on the first start.
pub const DEFAULT_WORKSPACE: &str = "My Workspace";

/// Sidebar tree geometry, in pixels.
const TREE_ROW_HEIGHT: f32 = 26.;
const TREE_INDENT: f32 = 16.;
const TREE_PAD: f32 = 4.;

/// Panel size limits, in pixels: below them the content runs into itself.
const SIDEBAR_WIDTH: Range<f32> = 220.0..480.0;
const MAIN_MIN_WIDTH: f32 = 420.;
const REQUEST_MIN_HEIGHT: f32 = 120.;
const RESPONSE_MIN_HEIGHT: f32 = 120.;

/// Heights of the URL bar and of the request tabs below it, in pixels. The
/// sidebar header spans both, so its bottom line runs on into the tabs' underline.
const URL_BAR_HEIGHT: f32 = 48.;
const TAB_ROW_HEIGHT: f32 = 30.;

/// The smallest window that still fits every panel at its minimum size.
pub const WINDOW_MIN_SIZE: (f32, f32) = (SIDEBAR_WIDTH.start + MAIN_MIN_WIDTH, 360.);

type MethodSelect = SelectState<Vec<&'static str>>;

/// A collection of the open workspace, as loaded from disk.
struct Collection {
    name: SharedString,
    dir: PathBuf,
    /// Created in the app and stored in the workspace, rather than added from elsewhere.
    owned: bool,
    entries: Vec<Entry>,
    folders: Vec<PathBuf>,
    /// Overrides for the workspace environments.
    envs: Environments,
}

/// A request in the sidebar: indices of its collection and of its entry there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Sel {
    col: usize,
    ix: usize,
}

/// Something in the sidebar that can be renamed in place.
#[derive(Debug, Clone, PartialEq)]
enum Target {
    Request(PathBuf),
    Folder(PathBuf),
    Collection(usize),
}

/// A sidebar item whose name is being edited in place.
struct Renaming {
    target: Target,
    input: Entity<InputState>,
    _subscription: Subscription,
}

/// A sidebar item being dragged to another folder or collection.
#[derive(Debug, Clone)]
enum Dragged {
    Request(PathBuf),
    Folder(PathBuf),
}

/// What follows the cursor while dragging: the item's label.
struct DragPreview {
    label: SharedString,
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            // Drawn outside the root view, so it does not inherit the UI font.
            .font_family(theme.font_family.clone())
            .text_size(px(13.))
            .shadow_md()
            .child(self.label.clone())
    }
}

/// One line of the sidebar tree.
enum Row {
    Collection(usize),
    Folder {
        col: usize,
        path: PathBuf,
        depth: usize,
    },
    Request {
        sel: Sel,
        depth: usize,
    },
}

/// Result of the last send.
enum Outcome {
    Response(Response),
    Error(SharedString),
}

pub struct ReqoApp {
    /// Keeps keyboard shortcuts working when no input is focused.
    focus_handle: FocusHandle,
    /// Where workspaces are stored.
    data_root: PathBuf,
    workspace: Workspace,
    /// All workspaces, for the switcher.
    workspaces: Vec<Workspace>,
    workspace_envs: Environments,
    collections: Vec<Collection>,
    /// Collections and folders shown folded in the sidebar.
    collapsed: HashSet<PathBuf>,
    selected: Option<Sel>,
    /// The editors hold changes that are not written to the selected file yet.
    dirty: bool,
    load_errors: Vec<SharedString>,
    renaming: Option<Renaming>,

    /// The environment whose variables fill `{{name}}` placeholders.
    env: Option<String>,
    /// Set when "No environment" was picked, so reloads do not pick one again.
    env_off: bool,

    method: Entity<MethodSelect>,
    url: Entity<InputState>,
    headers: Entity<TextareaState>,
    body: Entity<EditorState>,
    request_tab: usize,

    response_body: Entity<EditorState>,
    response_headers: Entity<TextareaState>,
    response_tab: usize,
    outcome: Option<Outcome>,
    in_flight: Option<Task<()>>,

    settings: Entity<Settings>,

    /// Keeps the app in step with the open environments editor.
    env_editor_subscription: Option<Subscription>,

    _subscriptions: Vec<Subscription>,
}

impl ReqoApp {
    /// Opens `workspace`. `reveal` is a collection folder to expand and select a request in.
    pub fn new(
        data_root: PathBuf,
        workspace: Workspace,
        reveal: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // First, so everything below is created with the saved theme.
        let settings = cx.new(|cx| Settings::new(window, cx));
        let method =
            cx.new(|cx| SelectState::new(METHODS.to_vec(), Some(IndexPath::default()), window, cx));
        let url =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://api.example.com/users"));
        let headers =
            cx.new(|cx| TextareaState::new(window, cx).placeholder("Accept: application/json"));
        let body = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("json")
                .line_number(true)
                .placeholder("Request body")
        });
        let response_body = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("json")
                .line_number(true)
                .folding(true)
                .searchable(true)
        });
        let response_headers = cx.new(|cx| TextareaState::new(window, cx));

        let mut subscriptions = vec![cx.subscribe(
            &method,
            |this, _, _: &SelectEvent<Vec<&'static str>>, cx| this.mark_dirty(cx),
        )];
        subscriptions.push(cx.subscribe(&url, Self::on_input_event));
        subscriptions.push(cx.subscribe(&headers, Self::on_input_event));
        subscriptions.push(cx.subscribe(&body, Self::on_input_event));

        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);

        let mut this = Self {
            focus_handle,
            data_root,
            workspace,
            workspaces: Vec::new(),
            workspace_envs: Environments::default(),
            collections: Vec::new(),
            collapsed: HashSet::new(),
            selected: None,
            dirty: false,
            load_errors: Vec::new(),
            renaming: None,
            env: None,
            env_off: false,
            method,
            url,
            headers,
            body,
            request_tab: 0,
            response_body,
            response_headers,
            response_tab: 0,
            outcome: None,
            in_flight: None,
            settings,
            env_editor_subscription: None,
            _subscriptions: subscriptions,
        };
        this.load(None, window, cx);
        if let Some(col) = reveal.and_then(|dir| this.collections.iter().position(|c| c.dir == dir))
        {
            this.collapsed.remove(&this.collections[col].dir);
            if !this.collections[col].entries.is_empty() {
                this.select(Sel { col, ix: 0 }, window, cx);
            }
        }
        this
    }

    fn on_input_event<E>(&mut self, _: Entity<E>, event: &InputEvent, cx: &mut Context<Self>) {
        if matches!(event, InputEvent::Change) {
            self.mark_dirty(cx);
        }
    }

    fn mark_dirty(&mut self, cx: &mut Context<Self>) {
        if self.selected.is_some() && !self.dirty {
            self.dirty = true;
            cx.notify();
        }
    }

    fn entry(&self, sel: Sel) -> &Entry {
        &self.collections[sel.col].entries[sel.ix]
    }

    fn find(&self, path: &Path) -> Option<Sel> {
        self.collections.iter().enumerate().find_map(|(col, c)| {
            let ix = c.entries.iter().position(|e| e.path == path)?;
            Some(Sel { col, ix })
        })
    }

    fn selected_path(&self) -> Option<PathBuf> {
        self.selected.map(|sel| self.entry(sel).path.clone())
    }

    /// Re-reads request files and environments from disk.
    fn reload(&mut self, _: &Reload, window: &mut Window, cx: &mut Context<Self>) {
        self.save(&SaveRequest, window, cx);
        let selected = self.selected_path();
        self.load(selected, window, cx);
    }

    /// Reads the workspace, its collections and environments from disk, then
    /// selects the request at `select`, or the first one.
    fn load(&mut self, select: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let mut errors: Vec<SharedString> = Vec::new();
        match Workspace::load(&self.workspace.dir) {
            Ok(workspace) => self.workspace = workspace,
            Err(err) => errors.push(format!("{err:#}").into()),
        }
        self.workspaces = workspace::list(&self.data_root);
        self.workspace_envs = Environments::load(&self.workspace.dir).unwrap_or_else(|err| {
            errors.push(format!("{err:#}").into());
            Environments::default()
        });

        self.collections = self
            .workspace
            .collections
            .iter()
            .map(|c| {
                let dir = self.workspace.collection_dir(c);
                if !dir.is_dir() {
                    errors.push(format!("{}: {} not found", c.name, dir.display()).into());
                }
                let (entries, load_errors) = collection::load(&dir);
                errors.extend(load_errors.iter().map(|e| format!("{e:#}").into()));
                let envs = Environments::load(&dir).unwrap_or_else(|err| {
                    errors.push(format!("{err:#}").into());
                    Environments::default()
                });
                Collection {
                    name: c.name.clone().into(),
                    folders: collection::folders(&dir),
                    owned: self.workspace.owns(c),
                    dir,
                    entries,
                    envs,
                }
            })
            .collect();
        self.load_errors = errors;

        let sel = select.and_then(|path| self.find(&path)).or_else(|| {
            self.collections
                .iter()
                .position(|c| !c.entries.is_empty())
                .map(|col| Sel { col, ix: 0 })
        });
        self.selected = None;
        self.dirty = false;
        match sel {
            Some(sel) => self.select(sel, window, cx),
            None => self.refresh_envs(),
        }
        cx.notify();
    }

    fn select(&mut self, sel: Sel, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected == Some(sel) {
            return;
        }
        self.save(&SaveRequest, window, cx);
        let collection_changed = self.selected.map(|s| s.col) != Some(sel.col);
        self.selected = Some(sel);
        self.outcome = None;
        self.in_flight = None;

        let request = self.entry(sel).request.clone();
        let method = METHODS
            .iter()
            .copied()
            .find(|m| m.eq_ignore_ascii_case(&request.method))
            .unwrap_or(METHODS[0]);
        self.method.update(cx, |state, cx| {
            state.set_selected_value(&method, window, cx)
        });
        self.url.update(cx, |state, cx| {
            state.set_value(request.url.clone(), window, cx)
        });
        self.headers.update(cx, |state, cx| {
            state.set_value(request.headers_text(), window, cx)
        });
        self.body.update(cx, |state, cx| {
            state.set_value(request.body.clone(), window, cx)
        });
        self.dirty = false;
        if collection_changed {
            self.refresh_envs();
        }
        cx.notify();
    }

    /// Workspace environments with the selected request's collection on top.
    fn envs(&self) -> Environments {
        match self.selected {
            Some(sel) => self.workspace_envs.merged(&self.collections[sel.col].envs),
            None => self.workspace_envs.clone(),
        }
    }

    /// Lists the environments available to the selected request, keeping the
    /// chosen one when it is still there.
    fn refresh_envs(&mut self) {
        let envs = self.envs();
        if let Some(env) = &self.env
            && envs.variables(env).is_none()
        {
            self.env = None;
        }
        if self.env.is_none() && !self.env_off {
            self.env = envs.names().next().map(str::to_string);
        }
    }

    fn set_env(&mut self, env: Option<String>, cx: &mut Context<Self>) {
        self.env_off = env.is_none();
        self.env = env;
        cx.notify();
    }

    /// The request as currently shown in the editors.
    fn edited_request(&self, sel: Sel, cx: &App) -> Request {
        Request {
            name: self.entry(sel).request.name.clone(),
            method: self
                .method
                .read(cx)
                .selected_value()
                .copied()
                .unwrap_or(METHODS[0])
                .to_string(),
            url: self.url.read(cx).value().trim().to_string(),
            headers: Request::parse_headers(&self.headers.read(cx).value()),
            body: self.body.read(cx).value().to_string(),
        }
    }

    fn save(&mut self, _: &SaveRequest, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sel) = self.selected else { return };
        if !self.dirty {
            return;
        }
        let request = self.edited_request(sel, cx);
        let entry = &mut self.collections[sel.col].entries[sel.ix];
        match request.save(&entry.path) {
            Ok(()) => {
                entry.request = request;
                self.dirty = false;
            }
            Err(err) => self.show_error(format!("{err:#}"), window, cx),
        }
        cx.notify();
    }

    /// Keeps a collection's requests in path order, as `collection::load` returns them.
    fn sort_entries(&mut self, col: usize) {
        let selected = self.selected_path();
        self.collections[col]
            .entries
            .sort_by(|a, b| a.path.cmp(&b.path));
        self.selected = selected.and_then(|path| self.find(&path));
    }

    /// Unfolds `path` and the folders above it.
    fn expand(&mut self, path: &Path) {
        self.collapsed.retain(|folded| !path.starts_with(folded));
    }

    fn toggle_folded(&mut self, path: &Path, cx: &mut Context<Self>) {
        if !self.collapsed.remove(path) {
            self.collapsed.insert(path.to_path_buf());
        }
        cx.notify();
    }

    /// Adds a request next to the selected one, or to the first collection.
    fn new_request(&mut self, _: &NewRequest, window: &mut Window, cx: &mut Context<Self>) {
        let target = match self.selected {
            Some(sel) => {
                let path = &self.entry(sel).path;
                let dir = path.parent().map(Path::to_path_buf);
                dir.map(|dir| (sel.col, dir))
            }
            None => self.collections.first().map(|c| (0, c.dir.clone())),
        };
        match target {
            Some((col, dir)) => self.new_request_in(col, &dir, window, cx),
            None => self.new_collection(window, cx),
        }
    }

    fn new_request_in(
        &mut self,
        col: usize,
        dir: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save(&SaveRequest, window, cx);
        match collection::create(dir) {
            Ok(entry) => {
                let path = entry.path.clone();
                self.collections[col].entries.push(entry);
                self.sort_entries(col);
                self.expand(dir);
                if let Some(sel) = self.find(&path) {
                    self.select(sel, window, cx);
                }
                self.start_rename(Target::Request(path), window, cx);
            }
            Err(err) => notify_error(err, window, cx),
        }
    }

    fn new_folder_in(
        &mut self,
        col: usize,
        parent: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match collection::create_folder(parent) {
            Ok(path) => {
                let folders = &mut self.collections[col].folders;
                folders.push(path.clone());
                folders.sort();
                self.expand(parent);
                self.start_rename(Target::Folder(path), window, cx);
            }
            Err(err) => notify_error(err, window, cx),
        }
    }

    fn new_collection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save(&SaveRequest, window, cx);
        match self.workspace.create_collection("New Collection") {
            Ok(ix) => {
                let selected = self.selected_path();
                self.load(selected, window, cx);
                self.start_rename(Target::Collection(ix), window, cx);
            }
            Err(err) => notify_error(err, window, cx),
        }
    }

    /// Moves a dragged request or folder into `dir`, then reloads so paths stay right.
    fn move_into(
        &mut self,
        dragged: &Dragged,
        dir: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save(&SaveRequest, window, cx);
        let selected = self.selected_path();
        let (from, result) = match dragged {
            Dragged::Request(path) => {
                let Some(sel) = self.find(path) else { return };
                (
                    path.clone(),
                    collection::move_entry(self.entry(sel), dir).map(|e| e.path),
                )
            }
            Dragged::Folder(path) => (path.clone(), collection::move_folder(path, dir)),
        };
        match result {
            Ok(to) if to == from => {}
            Ok(to) => {
                let moved = |p: PathBuf| match p.strip_prefix(&from) {
                    Ok(rest) if rest.as_os_str().is_empty() => to.clone(),
                    Ok(rest) => to.join(rest),
                    Err(_) => p,
                };
                self.collapsed = self.collapsed.drain().map(moved).collect();
                self.expand(dir);
                self.load(selected.map(moved), window, cx);
            }
            Err(err) => notify_error(err, window, cx),
        }
    }

    /// Asks for a folder and adds it to the workspace as a collection.
    fn add_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add as collection".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let dir = match paths.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Err(err)) => {
                    this.update_in(cx, |_, window, cx| notify_error(err, window, cx))
                        .ok();
                    None
                }
                _ => None,
            };
            if let Some(dir) = dir {
                this.update_in(cx, |this, window, cx| this.add_collection(&dir, window, cx))
                    .ok();
            }
        })
        .detach();
    }

    fn add_collection(&mut self, dir: &Path, window: &mut Window, cx: &mut Context<Self>) {
        self.save(&SaveRequest, window, cx);
        match self.workspace.add_collection(dir) {
            Ok(col) => {
                self.load(self.selected_path(), window, cx);
                let collection = &self.collections[col];
                self.collapsed.remove(&collection.dir.clone());
                if !collection.entries.is_empty() {
                    self.select(Sel { col, ix: 0 }, window, cx);
                }
            }
            Err(err) => notify_error(err, window, cx),
        }
    }

    fn delete_selected(&mut self, _: &DeleteRequest, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(sel) = self.selected {
            let path = self.entry(sel).path.clone();
            self.confirm_delete_request(path, window, cx);
        }
    }

    fn rename_selected(&mut self, _: &RenameRequest, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(sel) = self.selected {
            let path = self.entry(sel).path.clone();
            self.start_rename(Target::Request(path), window, cx);
        }
    }

    /// Replaces the item's name in the sidebar with an input.
    /// Enter or leaving the input commits, Escape cancels.
    fn start_rename(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        let name = match &target {
            Target::Request(path) => self
                .find(path)
                .map(|sel| self.entry(sel).request.name.clone()),
            Target::Folder(path) => file_name(path),
            Target::Collection(col) => self.collections.get(*col).map(|c| c.name.to_string()),
        };
        let Some(name) = name else { return };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
        input.update(cx, |state, cx| {
            state.focus(window, cx);
            state.select_all(window, cx);
        });
        let subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.finish_rename(true, window, cx);
            }
        });
        self.renaming = Some(Renaming {
            target,
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn finish_rename(&mut self, commit: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(renaming) = self.renaming.take() else {
            return;
        };
        // The input is going away; keep shortcuts working.
        self.focus_handle.focus(window, cx);
        cx.notify();
        let name = renaming.input.read(cx).value().trim().to_string();
        if !commit || name.is_empty() {
            return;
        }
        match renaming.target {
            Target::Request(path) => {
                let Some(sel) = self.find(&path) else { return };
                if name == self.entry(sel).request.name {
                    return;
                }
                // Keep unsaved edits: the rename rewrites the file from the saved request.
                if self.selected == Some(sel) {
                    self.save(&SaveRequest, window, cx);
                }
                match collection::rename(self.entry(sel), &name) {
                    Ok(entry) => {
                        self.collections[sel.col].entries[sel.ix] = entry;
                        self.sort_entries(sel.col);
                    }
                    Err(err) => notify_error(err, window, cx),
                }
            }
            Target::Folder(path) => {
                if file_name(&path).as_deref() == Some(name.as_str()) {
                    return;
                }
                self.save(&SaveRequest, window, cx);
                let selected = self.selected_path();
                match collection::rename_folder(&path, &name) {
                    Ok(new_path) => {
                        let moved = |p: PathBuf| match p.strip_prefix(&path) {
                            Ok(rest) => new_path.join(rest),
                            Err(_) => p,
                        };
                        self.collapsed = self.collapsed.drain().map(moved).collect();
                        self.load(selected.map(moved), window, cx);
                    }
                    Err(err) => notify_error(err, window, cx),
                }
            }
            Target::Collection(col) => {
                if name == self.collections[col].name.as_ref() {
                    return;
                }
                self.save(&SaveRequest, window, cx);
                let old_dir = self.collections[col].dir.clone();
                let selected = self.selected_path();
                if let Err(err) = self.workspace.rename_collection(col, &name) {
                    notify_error(err, window, cx);
                    return;
                }
                // An owned collection's folder follows its name.
                let new_dir = self
                    .workspace
                    .collection_dir(&self.workspace.collections[col]);
                if new_dir == old_dir {
                    self.collections[col].name = name.into();
                    return;
                }
                let moved = |p: PathBuf| match p.strip_prefix(&old_dir) {
                    Ok(rest) => new_dir.join(rest),
                    Err(_) => p,
                };
                self.collapsed = self.collapsed.drain().map(moved).collect();
                self.load(selected.map(moved), window, cx);
            }
        }
    }

    /// Asks before running `on_ok`.
    fn confirm(
        &mut self,
        title: String,
        description: String,
        ok_text: &'static str,
        on_ok: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let this = cx.entity().downgrade();
        let (title, description) = (SharedString::from(title), SharedString::from(description));
        let on_ok = Rc::new(on_ok);
        window.open_alert_dialog(cx, move |alert, _, _| {
            let this = this.clone();
            let on_ok = on_ok.clone();
            alert
                .title(title.clone())
                .description(description.clone())
                // `button_props` replaces all button settings, so `show_cancel` goes after it.
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(ok_text)
                        .ok_variant(ButtonVariant::Danger),
                )
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    this.update(cx, |this, cx| on_ok(this, window, cx)).ok();
                    true
                })
        });
    }

    /// Asks for a name, then runs `on_ok` with it.
    fn prompt_name(
        &mut self,
        title: &'static str,
        initial: String,
        ok_text: &'static str,
        on_ok: impl Fn(&mut Self, String, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(initial));
        // Enter in the input confirms the dialog like the OK button.
        let on_ok = Rc::new(on_ok);
        // The dialog takes focus when it opens; hand it to the input once it is shown.
        window.on_next_frame({
            let input = input.clone();
            move |window, cx| {
                input.update(cx, |state, cx| {
                    state.focus(window, cx);
                    state.select_all(window, cx);
                })
            }
        });
        let this = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let this = this.clone();
            let on_ok = on_ok.clone();
            let name_input = input.clone();
            alert
                .title(title)
                .child(div().w_full().child(Input::new(&input)))
                .button_props(DialogButtonProps::default().ok_text(ok_text))
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    let name = name_input.read(cx).value().trim().to_string();
                    if name.is_empty() {
                        return false;
                    }
                    this.update(cx, |this, cx| on_ok(this, name, window, cx))
                        .ok();
                    true
                })
        });
    }

    fn confirm_delete_request(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(sel) = self.find(&path) else { return };
        let collection = &self.collections[sel.col];
        let title = format!("Delete “{}”?", self.entry(sel).request.name);
        let file = format!(
            "{} will be removed from disk.",
            path.strip_prefix(&collection.dir)
                .unwrap_or(&path)
                .display()
        );
        self.confirm(
            title,
            file,
            "Delete",
            move |this, window, cx| this.delete_request(&path, window, cx),
            window,
            cx,
        );
    }

    fn delete_request(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sel) = self.find(path) else { return };
        if let Err(err) = collection::delete(self.entry(sel)) {
            notify_error(err, window, cx);
            return;
        }
        let was_selected = self.selected == Some(sel);
        let selected = if was_selected {
            None
        } else {
            self.selected_path()
        };
        self.collections[sel.col].entries.remove(sel.ix);
        self.selected = selected.and_then(|path| self.find(&path));
        if was_selected {
            // The selected request is gone: its unsaved edits go with it.
            self.dirty = false;
            let left = self.collections[sel.col].entries.len();
            if left > 0 {
                self.select(
                    Sel {
                        col: sel.col,
                        ix: sel.ix.min(left - 1),
                    },
                    window,
                    cx,
                );
            }
        }
        cx.notify();
    }

    fn confirm_delete_folder(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let requests = self
            .collections
            .iter()
            .flat_map(|c| &c.entries)
            .filter(|e| e.path.starts_with(&path))
            .count();
        let name = file_name(&path).unwrap_or_default();
        self.confirm(
            format!("Delete folder “{name}”?"),
            format!(
                "The folder and {} in it will be removed from disk.",
                plural(requests, "request")
            ),
            "Delete",
            move |this, window, cx| {
                if let Err(err) = collection::delete_folder(&path) {
                    notify_error(err, window, cx);
                    return;
                }
                let selected = this.selected_path().filter(|p| !p.starts_with(&path));
                this.save(&SaveRequest, window, cx);
                this.load(selected, window, cx);
            },
            window,
            cx,
        );
    }

    fn confirm_remove_collection(
        &mut self,
        col: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let collection = &self.collections[col];
        let dir = collection.dir.clone();
        let (title, description, ok) = if collection.owned {
            (
                format!("Delete collection “{}”?", collection.name),
                format!(
                    "The collection and {} in it will be removed from disk.",
                    plural(collection.entries.len(), "request")
                ),
                "Delete",
            )
        } else {
            (
                format!("Remove “{}” from the workspace?", collection.name),
                format!("The folder {} stays on disk.", dir.display()),
                "Remove",
            )
        };
        self.confirm(
            title,
            description,
            ok,
            move |this, window, cx| {
                let Some(col) = this.collections.iter().position(|c| c.dir == dir) else {
                    return;
                };
                this.save(&SaveRequest, window, cx);
                let selected = this.selected_path().filter(|p| !p.starts_with(&dir));
                if let Err(err) = this.workspace.remove_collection(col) {
                    notify_error(err, window, cx);
                }
                this.load(selected, window, cx);
            },
            window,
            cx,
        );
    }

    fn switch_workspace(&mut self, dir: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if dir == self.workspace.dir {
            return;
        }
        self.save(&SaveRequest, window, cx);
        match Workspace::load(dir) {
            Ok(workspace) => self.open_workspace(workspace, window, cx),
            Err(err) => notify_error(err, window, cx),
        }
    }

    fn open_workspace(
        &mut self,
        workspace: Workspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(err) = workspace::remember(&self.data_root, &workspace) {
            notify_error(err, window, cx);
        }
        self.workspace = workspace;
        self.collapsed.clear();
        self.renaming = None;
        self.load(None, window, cx);
    }

    fn new_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_name(
            "New workspace",
            String::new(),
            "Create",
            |this, name, window, cx| {
                this.save(&SaveRequest, window, cx);
                match workspace::create(&this.data_root, &name) {
                    Ok(workspace) => this.open_workspace(workspace, window, cx),
                    Err(err) => notify_error(err, window, cx),
                }
            },
            window,
            cx,
        );
    }

    fn rename_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_name(
            "Rename workspace",
            self.workspace.name.clone(),
            "Rename",
            |this, name, window, cx| {
                this.workspace.name = name;
                if let Err(err) = this.workspace.save() {
                    notify_error(err, window, cx);
                }
                this.workspaces = workspace::list(&this.data_root);
                cx.notify();
            },
            window,
            cx,
        );
    }

    fn confirm_delete_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let owned = self.collections.iter().filter(|c| c.owned).count();
        let external = self.collections.len() - owned;
        let mut description = format!(
            "Its environments and {} created in it will be removed from disk.",
            plural(owned, "collection")
        );
        if external > 0 {
            description.push_str(" Folders added from elsewhere stay.");
        }
        self.confirm(
            format!("Delete workspace “{}”?", self.workspace.name),
            description,
            "Delete",
            |this, window, cx| {
                // Its requests are about to go: drop unsaved edits instead of writing them.
                this.dirty = false;
                if let Err(err) = workspace::delete(&this.workspace) {
                    notify_error(err, window, cx);
                    return;
                }
                match workspace::open_last(&this.data_root, DEFAULT_WORKSPACE) {
                    Ok(workspace) => this.open_workspace(workspace, window, cx),
                    Err(err) => notify_error(err, window, cx),
                }
            },
            window,
            cx,
        );
    }

    /// Opens the environments editor on the workspace's or a collection's environments.
    fn edit_environments(&mut self, dir: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let mut scopes = vec![Scope {
            label: format!("Workspace “{}”", self.workspace.name).into(),
            dir: self.workspace.dir.clone(),
            hint: "Shared by every collection in the workspace.".into(),
        }];
        scopes.extend(self.collections.iter().map(|c| {
            Scope {
                label: format!("Collection “{}”", c.name).into(),
                dir: c.dir.clone(),
                hint: "Added to the workspace's environments. In an environment with the same \
                   name, variables set here win."
                    .into(),
            }
        }));
        let scope = scopes.iter().position(|s| s.dir == dir).unwrap_or(0);
        let editor = cx.new(|cx| EnvEditor::new(scopes, scope, window, cx));
        self.env_editor_subscription = Some(cx.subscribe_in(
            &editor,
            window,
            |this, _, EnvEditorEvent::Changed(dir), window, cx| this.reload_envs(dir, window, cx),
        ));
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Environments")
                .w(px(780.))
                .child(editor.clone())
        });
    }

    /// Re-reads the environments file in `dir`, the workspace's or a collection's.
    fn reload_envs(&mut self, dir: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let envs = Environments::load(dir).unwrap_or_else(|err| {
            notify_error(err, window, cx);
            Environments::default()
        });
        if dir == self.workspace.dir {
            self.workspace_envs = envs;
        } else if let Some(collection) = self.collections.iter_mut().find(|c| c.dir == dir) {
            collection.envs = envs;
        }
        self.refresh_envs();
        cx.notify();
    }

    fn variables(&self) -> Variables {
        self.env
            .as_ref()
            .and_then(|name| self.envs().variables(name).cloned())
            .unwrap_or_default()
    }

    fn send(&mut self, _: &SendRequest, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sel) = self.selected else { return };
        self.save(&SaveRequest, window, cx);
        let request = self.edited_request(sel, cx);
        let vars = self.variables();

        self.in_flight = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { reqo_core::execute(&request, &vars) })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.in_flight = None;
                match result {
                    Ok(response) => this.show_response(response, window, cx),
                    Err(err) => this.show_error(format!("{err:#}"), window, cx),
                }
            })
            .ok();
        }));
        cx.notify();
    }

    fn show_response(&mut self, response: Response, window: &mut Window, cx: &mut Context<Self>) {
        let language = if response.is_json { "json" } else { "text" };
        self.response_body.update(cx, |state, cx| {
            state.set_highlighter(language, cx);
            state.set_value(response.body.clone(), window, cx);
        });
        self.response_headers.update(cx, |state, cx| {
            state.set_value(response.headers_text(), window, cx)
        });
        self.outcome = Some(Outcome::Response(response));
        cx.notify();
    }

    fn show_error(&mut self, message: String, _: &mut Window, cx: &mut Context<Self>) {
        self.outcome = Some(Outcome::Error(message.into()));
        cx.notify();
    }

    fn toggle_theme(&mut self, _: &ToggleTheme, window: &mut Window, cx: &mut Context<Self>) {
        self.settings
            .update(cx, |settings, cx| settings.toggle_mode(window, cx));
        cx.notify();
    }

    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        let settings = self.settings.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            dialog.title("Settings").w(px(520.)).child(settings.clone())
        });
    }

    /// The sidebar tree in display order, skipping the contents of folded items.
    fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for (col, collection) in self.collections.iter().enumerate() {
            rows.push(Row::Collection(col));
            if !self.collapsed.contains(&collection.dir) {
                self.push_folder_rows(col, &collection.dir, 1, &mut rows);
            }
        }
        rows
    }

    /// Subfolders first, then requests, like a file manager.
    fn push_folder_rows(&self, col: usize, dir: &Path, depth: usize, rows: &mut Vec<Row>) {
        let collection = &self.collections[col];
        for folder in collection
            .folders
            .iter()
            .filter(|f| f.parent() == Some(dir))
        {
            rows.push(Row::Folder {
                col,
                path: folder.clone(),
                depth,
            });
            if !self.collapsed.contains(folder) {
                self.push_folder_rows(col, folder, depth + 1, rows);
            }
        }
        for (ix, entry) in collection.entries.iter().enumerate() {
            if entry.path.parent() == Some(dir) {
                rows.push(Row::Request {
                    sel: Sel { col, ix },
                    depth,
                });
            }
        }
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let rows = self.rows();

        v_flex()
            .size_full()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .child(
                // The workspace and its environment read as a title with a
                // subtitle; the actions share the title's line.
                v_flex()
                    .h(px(URL_BAR_HEIGHT + TAB_ROW_HEIGHT))
                    .flex_none()
                    .justify_center()
                    .gap_0p5()
                    .px_1()
                    .border_b_1()
                    .border_color(theme.sidebar_border)
                    .child(
                        h_flex()
                            .gap_1()
                            .justify_between()
                            .child(div().min_w_0().child(self.render_workspace_menu(cx)))
                            .child(
                                h_flex()
                                    .child(
                                        Button::new("reload")
                                            .cursor_pointer()
                                            .ghost()
                                            .small()
                                            .icon(IconName::RefreshCw)
                                            .tooltip("Reload from disk (Ctrl+R)")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.reload(&Reload, window, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("settings")
                                            .cursor_pointer()
                                            .ghost()
                                            .small()
                                            .icon(IconName::Settings)
                                            .tooltip("Settings (Ctrl+,)")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.open_settings(&OpenSettings, window, cx)
                                            })),
                                    ),
                            ),
                    )
                    .child(h_flex().min_w_0().child(self.render_env_picker(cx))),
            )
            .child(
                h_flex()
                    .pl_3()
                    .pr_1()
                    .py_1()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().muted_foreground)
                            .child("COLLECTIONS"),
                    )
                    .child(
                        h_flex()
                            .child(
                                Button::new("new-collection")
                                    .cursor_pointer()
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::FolderPlus)
                                    .tooltip("New collection")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.new_collection(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("add-folder")
                                    .cursor_pointer()
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::FolderInput)
                                    .tooltip("Add a folder as a collection")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.add_folder(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("new-request")
                                    .cursor_pointer()
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Plus)
                                    .tooltip("New request (Ctrl+N)")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.new_request(&NewRequest, window, cx)
                                    })),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .id("tree")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_1()
                    .pb_1()
                    .children(rows.into_iter().map(|row| match row {
                        Row::Collection(col) => self.render_collection(col, cx),
                        Row::Folder { col, path, depth } => self.render_folder(col, path, depth, cx),
                        Row::Request { sel, depth } => self.render_request_row(sel, depth, cx),
                    }))
                    .when(self.collections.is_empty(), |this| {
                        this.child(
                            div()
                                .p_2()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("No collections yet. Create one, or add a folder with request files."),
                        )
                    })
                    .child(self.render_tree_space(cx)),
            )
            .children(self.load_errors.iter().map(|error| {
                div()
                    .p_2()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(error.clone())
            }))
    }

    /// The empty space below the tree; right-click it for collection actions.
    fn render_tree_space(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        div()
            .id("tree-space")
            .flex_1()
            .min_h(px(48.))
            .context_menu(move |menu, _, _| {
                menu.item(menu_item(
                    &this,
                    "New collection",
                    Some(IconName::FolderPlus),
                    |this, window, cx| this.new_collection(window, cx),
                ))
                .item(menu_item(
                    &this,
                    "Add folder…",
                    Some(IconName::FolderInput),
                    |this, window, cx| this.add_folder(window, cx),
                ))
            })
    }

    /// Workspace name that opens the switcher and workspace actions.
    fn render_workspace_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let current = self.workspace.dir.clone();
        let workspaces: Vec<(SharedString, PathBuf)> = self
            .workspaces
            .iter()
            .map(|w| (w.name.clone().into(), w.dir.clone()))
            .collect();
        let workspace_dir = self.workspace.dir.clone();

        Button::new("workspace")
            .cursor_pointer()
            .ghost()
            .small()
            .icon(IconName::Layers)
            .label(SharedString::from(self.workspace.name.clone()))
            .font_weight(FontWeight::SEMIBOLD)
            .dropdown_caret(true)
            .tooltip("Switch workspace")
            .dropdown_menu(move |menu: PopupMenu, _, _| {
                let mut menu = menu.min_w(px(220.)).label("Workspaces");
                for (name, dir) in &workspaces {
                    let is_current = *dir == current;
                    let dir = dir.clone();
                    menu = menu.item(
                        menu_item(&this, name.clone(), None, move |this, window, cx| {
                            this.switch_workspace(&dir, window, cx)
                        })
                        .checked(is_current),
                    );
                }
                let env_dir = workspace_dir.clone();
                menu.separator()
                    .item(menu_item(
                        &this,
                        "New workspace…",
                        Some(IconName::Plus),
                        |this, window, cx| this.new_workspace(window, cx),
                    ))
                    .item(menu_item(
                        &this,
                        "Rename workspace…",
                        Some(IconName::Pencil),
                        |this, window, cx| this.rename_workspace(window, cx),
                    ))
                    .item(menu_item(
                        &this,
                        "Edit environments",
                        Some(IconName::Settings2),
                        move |this, window, cx| this.edit_environments(&env_dir, window, cx),
                    ))
                    .separator()
                    .item(menu_item(
                        &this,
                        "Delete workspace…",
                        Some(IconName::Trash),
                        |this, window, cx| this.confirm_delete_workspace(window, cx),
                    ))
            })
    }

    /// The row's label, or the rename input when `target` is being renamed.
    fn row_label(&self, target: &Target, label: SharedString) -> AnyElement {
        match &self.renaming {
            Some(renaming) if renaming.target == *target => div()
                .flex_1()
                // Selecting text with the mouse must not start dragging the row.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(Input::new(&renaming.input).xsmall())
                .into_any_element(),
            _ => div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(label)
                .into_any_element(),
        }
    }

    /// Common look of sidebar rows: fixed height, indented by `depth`, with a
    /// guide line for each level above.
    fn tree_row(
        &self,
        id: impl Into<ElementId>,
        depth: usize,
        target: &Target,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = cx.theme();
        let renaming = matches!(&self.renaming, Some(r) if r.target == *target);
        h_flex()
            .id(id)
            .relative()
            .h(px(TREE_ROW_HEIGHT))
            .flex_none()
            .gap_1()
            .pl(px(TREE_PAD + depth as f32 * TREE_INDENT))
            .pr_2()
            .rounded(theme.radius)
            .text_size(px(13.))
            .cursor_pointer()
            .hover(|this| this.bg(theme.list_hover))
            .children((0..depth).map(|level| {
                // Centered under the chevron of the ancestor at `level`.
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(TREE_PAD + level as f32 * TREE_INDENT + TREE_INDENT / 2.))
                    .w(px(1.))
                    .bg(theme.border)
            }))
            .when(renaming, |this| {
                this.capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        cx.stop_propagation();
                        this.finish_rename(false, window, cx);
                    }
                }))
            })
    }

    /// Lets requests and folders be dropped on the row, moving them into `dir`.
    fn drop_into(&self, row: Stateful<Div>, dir: PathBuf, cx: &mut Context<Self>) -> Stateful<Div> {
        row.drag_over::<Dragged>(|style, _, _, cx| style.bg(cx.theme().drop_target))
            .on_drop(cx.listener(move |this, dragged: &Dragged, window, cx| {
                this.move_into(dragged, &dir, window, cx)
            }))
    }

    /// New request and new folder buttons, shown while the row is hovered.
    fn hover_actions(
        &self,
        group: SharedString,
        col: usize,
        dir: PathBuf,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let folder_dir = dir.clone();
        h_flex()
            .flex_none()
            .opacity(0.)
            .group_hover(group.clone(), |style| style.opacity(1.))
            .child(
                Button::new(SharedString::from(format!("{group}:new-request")))
                    .cursor_pointer()
                    .ghost()
                    .xsmall()
                    .icon(IconName::Plus)
                    .tooltip("New request")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.new_request_in(col, &dir, window, cx)
                    })),
            )
            .child(
                Button::new(SharedString::from(format!("{group}:new-folder")))
                    .cursor_pointer()
                    .ghost()
                    .xsmall()
                    .icon(IconName::FolderPlus)
                    .tooltip("New folder")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.new_folder_in(col, &folder_dir, window, cx)
                    })),
            )
    }

    fn render_collection(&self, col: usize, cx: &mut Context<Self>) -> AnyElement {
        let collection = &self.collections[col];
        let dir = collection.dir.clone();
        let expanded = !self.collapsed.contains(&dir);
        let target = Target::Collection(col);
        let this = cx.entity().downgrade();
        let owned = collection.owned;
        let path_label = SharedString::from(dir.display().to_string());

        let id = SharedString::from(format!("collection:{}", dir.display()));
        let row = self.tree_row(id.clone(), 0, &target, cx).group(id.clone());
        self.drop_into(row, dir.clone(), cx)
            .when(col > 0, |this| this.mt_1())
            .font_weight(FontWeight::SEMIBOLD)
            .child(slot(chevron(expanded, cx)))
            .child(slot(
                Icon::new(IconName::LibraryBig)
                    .small()
                    .text_color(cx.theme().muted_foreground),
            ))
            .child(self.row_label(&target, collection.name.clone()))
            .child(self.hover_actions(id, col, dir.clone(), cx))
            .when(!owned, |this| {
                // Only the icon carries the path, so the tooltip stays clear of the context menu.
                this.child(
                    div()
                        .id(SharedString::from(format!(
                            "collection-path:{}",
                            dir.display()
                        )))
                        .child(
                            Icon::new(IconName::FolderSymlink)
                                .xsmall()
                                .text_color(cx.theme().muted_foreground),
                        )
                        .tooltip(move |window, cx| {
                            Tooltip::new(path_label.clone()).build(window, cx)
                        }),
                )
            })
            .on_click(cx.listener({
                let dir = dir.clone();
                move |this, event: &ClickEvent, window, cx| {
                    this.toggle_folded(&dir, cx);
                    if event.click_count() == 2 {
                        this.start_rename(Target::Collection(col), window, cx);
                    }
                }
            }))
            .context_menu(move |menu, _, _| {
                let (d1, d2, d3, d4) = (dir.clone(), dir.clone(), dir.clone(), dir.clone());
                menu.item(menu_item(
                    &this,
                    "New request",
                    Some(IconName::Plus),
                    move |this, window, cx| this.new_request_in(col, &d1, window, cx),
                ))
                .item(menu_item(
                    &this,
                    "New folder",
                    Some(IconName::FolderPlus),
                    move |this, window, cx| this.new_folder_in(col, &d2, window, cx),
                ))
                .separator()
                .item(menu_item(
                    &this,
                    "Rename",
                    Some(IconName::Pencil),
                    move |this, window, cx| this.start_rename(Target::Collection(col), window, cx),
                ))
                .item(menu_item(
                    &this,
                    "Edit environments",
                    Some(IconName::Settings2),
                    move |this, window, cx| this.edit_environments(&d3, window, cx),
                ))
                .item(menu_item(
                    &this,
                    "Show in file manager",
                    Some(IconName::FolderOpen),
                    move |_, _, cx| cx.reveal_path(&d4),
                ))
                .separator()
                .item(if owned {
                    menu_item(
                        &this,
                        "Delete collection",
                        Some(IconName::Trash),
                        move |this, window, cx| this.confirm_remove_collection(col, window, cx),
                    )
                } else {
                    menu_item(
                        &this,
                        "Remove from workspace",
                        Some(IconName::Unlink),
                        move |this, window, cx| this.confirm_remove_collection(col, window, cx),
                    )
                })
            })
            .into_any_element()
    }

    fn render_folder(
        &self,
        col: usize,
        path: PathBuf,
        depth: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expanded = !self.collapsed.contains(&path);
        let target = Target::Folder(path.clone());
        let name: SharedString = file_name(&path).unwrap_or_default().into();
        let this = cx.entity().downgrade();

        let id = SharedString::from(format!("folder:{}", path.display()));
        let row = self
            .tree_row(id.clone(), depth, &target, cx)
            .group(id.clone());
        let label = name.clone();
        self.drop_into(row, path.clone(), cx)
            .on_drag(Dragged::Folder(path.clone()), move |_, _, _, cx| {
                cx.new(|_| DragPreview {
                    label: label.clone(),
                })
            })
            .child(slot(chevron(expanded, cx)))
            .child(slot(
                Icon::new(if expanded {
                    IconName::FolderOpen
                } else {
                    IconName::Folder
                })
                .small()
                .text_color(cx.theme().muted_foreground),
            ))
            .child(self.row_label(&target, name))
            .child(self.hover_actions(id, col, path.clone(), cx))
            .on_click(cx.listener({
                let path = path.clone();
                move |this, event: &ClickEvent, window, cx| {
                    this.toggle_folded(&path, cx);
                    if event.click_count() == 2 {
                        this.start_rename(Target::Folder(path.clone()), window, cx);
                    }
                }
            }))
            .context_menu(move |menu, _, _| {
                let (p1, p2, p3, p4) = (path.clone(), path.clone(), path.clone(), path.clone());
                menu.item(menu_item(
                    &this,
                    "New request",
                    Some(IconName::Plus),
                    move |this, window, cx| this.new_request_in(col, &p1, window, cx),
                ))
                .item(menu_item(
                    &this,
                    "New folder",
                    Some(IconName::FolderPlus),
                    move |this, window, cx| this.new_folder_in(col, &p2, window, cx),
                ))
                .separator()
                .item(menu_item(
                    &this,
                    "Rename",
                    Some(IconName::Pencil),
                    move |this, window, cx| {
                        this.start_rename(Target::Folder(p3.clone()), window, cx)
                    },
                ))
                .item(menu_item(
                    &this,
                    "Delete",
                    Some(IconName::Trash),
                    move |this, window, cx| this.confirm_delete_folder(p4.clone(), window, cx),
                ))
            })
            .into_any_element()
    }

    fn render_request_row(&self, sel: Sel, depth: usize, cx: &mut Context<Self>) -> AnyElement {
        let entry = self.entry(sel);
        let selected = self.selected == Some(sel);
        let request = &entry.request;
        let name: SharedString = if request.name.is_empty() {
            "Untitled".into()
        } else {
            request.name.clone().into()
        };
        let path = entry.path.clone();
        let target = Target::Request(path.clone());
        let this = cx.entity().downgrade();

        let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let label = SharedString::from(format!("{} {}", method_label(&request.method), name));
        // The method takes the place of the chevron and icon of folders.
        let row = self.tree_row(
            SharedString::from(format!("request:{}", path.display())),
            depth,
            &target,
            cx,
        );
        self.drop_into(row, parent, cx)
            .on_drag(Dragged::Request(path.clone()), move |_, _, _, cx| {
                cx.new(|_| DragPreview {
                    label: label.clone(),
                })
            })
            .gap_1p5()
            .when(selected, |this| this.bg(cx.theme().list_active))
            .child(method_badge(&request.method, cx))
            .child(self.row_label(&target, name))
            .when(selected && self.dirty, |this| {
                this.child(div().text_color(cx.theme().muted_foreground).child("•"))
            })
            .on_click(cx.listener({
                let path = path.clone();
                move |this, event: &ClickEvent, window, cx| {
                    let Some(sel) = this.find(&path) else { return };
                    this.select(sel, window, cx);
                    this.focus_handle.focus(window, cx);
                    if event.click_count() == 2 {
                        this.start_rename(Target::Request(path.clone()), window, cx);
                    }
                }
            }))
            .context_menu(move |menu, _, _| {
                let (p1, p2) = (path.clone(), path.clone());
                menu.item(menu_item(
                    &this,
                    "Rename",
                    Some(IconName::Pencil),
                    move |this, window, cx| {
                        this.start_rename(Target::Request(p1.clone()), window, cx)
                    },
                ))
                .item(menu_item(
                    &this,
                    "Delete",
                    Some(IconName::Trash),
                    move |this, window, cx| this.confirm_delete_request(p2.clone(), window, cx),
                ))
            })
            .into_any_element()
    }

    fn render_url_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let sending = self.in_flight.is_some();
        h_flex()
            .h(px(URL_BAR_HEIGHT))
            .flex_none()
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .w(px(120.))
                    .flex_none()
                    .child(Select::new(&self.method).cursor_pointer()),
            )
            .child(div().flex_1().child(Input::new(&self.url)))
            .child(
                Button::new("send")
                    .cursor_pointer()
                    .primary()
                    .icon(IconName::Send)
                    // Only the icon turns into a spinner: a different label would
                    // change the button width and shift the whole URL bar.
                    .label("Send")
                    .loading(sending)
                    .tooltip("Send (Ctrl+Enter)")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.send(&SendRequest, window, cx)),
                    ),
            )
    }

    /// The environment button under the workspace menu: shows which
    /// environment of the workspace fills `{{variables}}` and switches it, or
    /// opens the editor.
    fn render_env_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let names: Vec<String> = self.envs().names().map(str::to_string).collect();
        let current = self.env.clone();
        let workspace_dir = self.workspace.dir.clone();
        let (icon, label, tooltip) = match &current {
            Some(env) => (
                IconName::Globe,
                env.clone(),
                "Environment: the values for {{variables}}",
            ),
            None => (
                IconName::GlobeOff,
                "No environment".to_string(),
                "No environment: {{variables}} stay as they are",
            ),
        };

        // Styled like the workspace switcher above it, but muted: it is the
        // workspace's subtitle.
        Button::new("environment")
            .cursor_pointer()
            .ghost()
            .small()
            .text_color(cx.theme().muted_foreground)
            .icon(icon)
            .label(label)
            .dropdown_caret(true)
            .tooltip(tooltip)
            .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu: PopupMenu, _, _| {
                let mut menu = menu.min_w(px(220.)).label("Environment");
                for name in &names {
                    let env = name.clone();
                    menu = menu.item(
                        menu_item(&this, name.clone(), None, move |this, _, cx| {
                            this.set_env(Some(env.clone()), cx)
                        })
                        .checked(current.as_deref() == Some(name)),
                    );
                }
                let dir = workspace_dir.clone();
                menu.separator()
                    .item(
                        menu_item(&this, "No environment", None, |this, _, cx| {
                            this.set_env(None, cx)
                        })
                        .checked(current.is_none()),
                    )
                    .separator()
                    .item(menu_item(
                        &this,
                        "Manage environments…",
                        Some(IconName::Settings2),
                        move |this, window, cx| this.edit_environments(&dir, window, cx),
                    ))
            })
    }

    fn render_request(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.request_tab {
            0 => editor(&self.body, cx).into_any_element(),
            _ => Textarea::new(&self.headers)
                .bordered(false)
                .h_full()
                .font_family(cx.theme().mono_font_family.clone())
                .text_size(cx.theme().mono_font_size)
                .into_any_element(),
        };
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(TAB_ROW_HEIGHT))
                    .flex_none()
                    // The underline sits on the bottom edge, whatever the tabs' own height.
                    .items_end()
                    .gap_2()
                    .px_2()
                    .justify_between()
                    .child(
                        TabBar::new("request-tabs")
                            .underline()
                            .small()
                            .selected_index(self.request_tab)
                            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                                this.request_tab = *ix;
                                cx.notify();
                            }))
                            .child(Tab::new().label("Body").cursor_pointer())
                            .child(Tab::new().label("Headers").cursor_pointer()),
                    ),
            )
            .child(div().flex_1().min_h_0().child(content))
    }

    fn render_response(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let summary = match &self.outcome {
            None if self.in_flight.is_some() => {
                div().text_color(theme.muted_foreground).child("Sending…")
            }
            None => div()
                .text_color(theme.muted_foreground)
                .child("Press Send or Ctrl+Enter"),
            Some(Outcome::Error(message)) => div()
                .text_color(theme.danger)
                .truncate()
                .child(message.clone()),
            Some(Outcome::Response(response)) => h_flex()
                .gap_3()
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(status_color(response.status, cx))
                        .child(format!("{} {}", response.status, response.reason)),
                )
                .child(
                    div()
                        .text_color(theme.muted_foreground)
                        .child(format_duration(response.elapsed)),
                )
                .child(
                    div()
                        .text_color(theme.muted_foreground)
                        .child(format_size(response.size)),
                ),
        };

        let content = match (&self.outcome, self.response_tab) {
            (Some(Outcome::Response(_)), 0) => editor(&self.response_body, cx)
                .readonly(true)
                .into_any_element(),
            (Some(Outcome::Response(_)), _) => Textarea::new(&self.response_headers)
                .readonly(true)
                .bordered(false)
                .h_full()
                .font_family(cx.theme().mono_font_family.clone())
                .text_size(cx.theme().mono_font_size)
                .into_any_element(),
            _ => div().into_any_element(),
        };

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .gap_4()
                    .px_2()
                    .justify_between()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        TabBar::new("response-tabs")
                            .underline()
                            .small()
                            .selected_index(self.response_tab)
                            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                                this.response_tab = *ix;
                                cx.notify();
                            }))
                            .child(Tab::new().label("Response").cursor_pointer())
                            .child(Tab::new().label("Headers").cursor_pointer()),
                    )
                    .child(div().text_sm().min_w_0().child(summary)),
            )
            .child(div().flex_1().min_h_0().child(content))
    }

    /// Shown instead of the editors when no request is selected.
    fn render_empty(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_collections = !self.collections.is_empty();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .text_color(cx.theme().muted_foreground)
            .child(if has_collections {
                "No request selected"
            } else {
                "This workspace has no collections yet"
            })
            .child(
                h_flex()
                    .gap_2()
                    .when(has_collections, |this| {
                        this.child(
                            Button::new("empty-new-request")
                                .cursor_pointer()
                                .icon(IconName::Plus)
                                .label("New request")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.new_request(&NewRequest, window, cx)
                                })),
                        )
                    })
                    .when(!has_collections, |this| {
                        this.child(
                            Button::new("empty-new-collection")
                                .cursor_pointer()
                                .icon(IconName::FolderPlus)
                                .label("New collection")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.new_collection(window, cx)
                                })),
                        )
                        .child(
                            Button::new("empty-add-folder")
                                .cursor_pointer()
                                .icon(IconName::FolderInput)
                                .label("Add folder…")
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.add_folder(window, cx)),
                                ),
                        )
                    }),
            )
    }
}

impl Render for ReqoApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let main = if self.selected.is_some() {
            v_flex()
                .size_full()
                .child(self.render_url_bar(cx))
                .child(
                    div().flex_1().min_h_0().child(
                        v_resizable("request-response")
                            .child(
                                resizable_panel()
                                    .size(px(300.))
                                    .size_range(px(REQUEST_MIN_HEIGHT)..Pixels::MAX)
                                    .flex_none()
                                    .child(self.render_request(cx)),
                            )
                            .child(
                                resizable_panel()
                                    .size_range(px(RESPONSE_MIN_HEIGHT)..Pixels::MAX)
                                    .child(self.render_response(cx)),
                            ),
                    ),
                )
                .into_any_element()
        } else {
            self.render_empty(cx).into_any_element()
        };

        div()
            .id("reqo")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(Self::send))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::new_request))
            .on_action(cx.listener(Self::reload))
            .on_action(cx.listener(Self::toggle_theme))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::rename_selected))
            .on_action(cx.listener(Self::delete_selected))
            .child(
                h_resizable("sidebar-main")
                    .child(
                        resizable_panel()
                            .size(px(260.))
                            .size_range(px(SIDEBAR_WIDTH.start)..px(SIDEBAR_WIDTH.end))
                            .flex_none()
                            .child(self.render_sidebar(cx)),
                    )
                    .child(
                        resizable_panel()
                            .size_range(px(MAIN_MIN_WIDTH)..Pixels::MAX)
                            .child(main),
                    ),
            )
            // gpui-kit's Root keeps overlays but leaves drawing them to the view.
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}

/// A menu item that runs `action` on the app.
fn menu_item(
    this: &WeakEntity<ReqoApp>,
    label: impl Into<SharedString>,
    icon: Option<IconName>,
    action: impl Fn(&mut ReqoApp, &mut Window, &mut Context<ReqoApp>) + 'static,
) -> PopupMenuItem {
    let this = this.clone();
    PopupMenuItem::new(label)
        .when_some(icon, |item, icon| item.icon(icon))
        .on_click(move |_, window, cx| {
            this.update(cx, |this, cx| action(this, window, cx)).ok();
        })
}

fn notify_error(err: anyhow::Error, window: &mut Window, cx: &mut App) {
    window.push_notification(Notification::error(format!("{err:#}")), cx);
}

/// A fixed-width cell for a chevron or icon, so labels line up across rows.
fn slot(child: impl IntoElement) -> Div {
    h_flex()
        .w(px(TREE_INDENT))
        .flex_none()
        .justify_center()
        .child(child)
}

/// Methods as short as they read, so the request names line up.
fn method_label(method: &str) -> SharedString {
    match method {
        "DELETE" => "DEL".into(),
        "OPTIONS" => "OPT".into(),
        method => SharedString::from(method.to_string()),
    }
}

fn chevron(expanded: bool, cx: &App) -> Icon {
    Icon::new(if expanded {
        IconName::ChevronDown
    } else {
        IconName::ChevronRight
    })
    .small()
    .text_color(cx.theme().muted_foreground)
}

fn file_name(path: &Path) -> Option<String> {
    path.file_name().map(|n| n.to_string_lossy().into_owned())
}

fn plural(n: usize, word: &str) -> String {
    match n {
        1 => format!("1 {word}"),
        n => format!("{n} {word}s"),
    }
}

fn editor(state: &Entity<EditorState>, cx: &App) -> Editor {
    Editor::new(state)
        .bordered(false)
        .h(relative(1.))
        .font_family(cx.theme().mono_font_family.clone())
        .text_size(cx.theme().mono_font_size)
}

/// The method as a fixed-width tinted badge: its left edge follows the tree
/// indent and the names after it line up.
fn method_badge(method: &str, cx: &App) -> Div {
    let color = method_color(method, cx);
    h_flex()
        .w(px(2. * TREE_INDENT + 6.))
        .h(px(16.))
        .flex_none()
        .justify_center()
        .rounded(px(3.))
        .bg(color.opacity(0.14))
        .font_family(cx.theme().mono_font_family.clone())
        .text_size(px(10.))
        .font_weight(FontWeight::BOLD)
        .text_color(color)
        .child(method_label(method))
}

fn method_color(method: &str, cx: &App) -> Hsla {
    let theme = cx.theme();
    match method {
        "GET" => theme.success,
        "POST" => theme.warning,
        "PUT" | "PATCH" => theme.info,
        "DELETE" => theme.danger,
        _ => theme.muted_foreground,
    }
}

fn status_color(status: u16, cx: &App) -> Hsla {
    let theme = cx.theme();
    match status {
        200..=299 => theme.success,
        300..=399 => theme.info,
        _ => theme.danger,
    }
}

fn format_duration(elapsed: std::time::Duration) -> String {
    let ms = elapsed.as_secs_f64() * 1000.0;
    if ms < 1000.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{:.2} s", ms / 1000.0)
    }
}

fn format_size(bytes: usize) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.1} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}
