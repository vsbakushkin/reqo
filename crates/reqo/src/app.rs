use std::path::{Path, PathBuf};

use crate::theme::{self, ThemeSettings};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, IndexPath, Root, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariant, ButtonVariants as _},
    dialog::DialogButtonProps,
    h_flex,
    input::{Editor, EditorState, Input, InputEvent, InputState, Textarea, TextareaState},
    menu::{ContextMenuExt as _, PopupMenuItem},
    resizable::{h_resizable, resizable_panel, v_resizable},
    select::{Select, SelectEvent, SelectState},
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use reqo_core::collection::{self, Entry};
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
        DeleteRequest
    ]
);

/// Key context of the app view; bindings in it apply when no input has focus.
pub const KEY_CONTEXT: &str = "Reqo";

type MethodSelect = SelectState<Vec<&'static str>>;
type EnvSelect = SelectState<Vec<SharedString>>;

/// A sidebar entry whose name is being edited in place.
struct Renaming {
    path: PathBuf,
    input: Entity<InputState>,
    _subscription: Subscription,
}

/// Result of the last send.
enum Outcome {
    Response(Response),
    Error(SharedString),
}

pub struct ReqoApp {
    /// Keeps keyboard shortcuts working when no input is focused.
    focus_handle: FocusHandle,
    dir: PathBuf,
    entries: Vec<Entry>,
    selected: Option<usize>,
    /// The editors hold changes that are not written to the selected file yet.
    dirty: bool,
    load_errors: Vec<SharedString>,
    renaming: Option<Renaming>,

    envs: Environments,
    env_select: Entity<EnvSelect>,

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

    theme_settings: ThemeSettings,
    theme_select: Entity<SelectState<Vec<SharedString>>>,

    _subscriptions: Vec<Subscription>,
}

impl ReqoApp {
    pub fn new(dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
        let env_select = cx.new(|cx| SelectState::new(Vec::new(), None, window, cx));

        let mut subscriptions = vec![cx.subscribe(
            &method,
            |this, _, _: &SelectEvent<Vec<&'static str>>, cx| this.mark_dirty(cx),
        )];
        subscriptions.push(cx.subscribe(&url, Self::on_input_event));
        subscriptions.push(cx.subscribe(&headers, Self::on_input_event));
        subscriptions.push(cx.subscribe(&body, Self::on_input_event));

        let theme_settings = ThemeSettings::load();
        theme_settings.apply(window, cx);
        subscriptions.push(cx.observe_window_appearance(window, |this, window, cx| {
            if this.theme_settings.follows_system() {
                this.theme_settings.apply(window, cx);
                this.sync_theme_select(window, cx);
            }
        }));
        let theme_select = cx.new(|cx| SelectState::new(Vec::new(), None, window, cx));
        subscriptions.push(cx.subscribe_in(
            &theme_select,
            window,
            |this, _, event: &SelectEvent<Vec<SharedString>>, window, cx| {
                let SelectEvent::Confirm(Some(name)) = event else {
                    return;
                };
                this.theme_settings.set_theme(name, window, cx);
                cx.notify();
            },
        ));

        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);

        let mut this = Self {
            focus_handle,
            dir,
            entries: Vec::new(),
            selected: None,
            dirty: false,
            load_errors: Vec::new(),
            envs: Environments::default(),
            env_select,
            renaming: None,
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
            theme_settings,
            theme_select,
            _subscriptions: subscriptions,
        };
        this.sync_theme_select(window, cx);
        this.reload(&Reload, window, cx);
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

    /// Re-reads request files and environments from disk.
    fn reload(&mut self, _: &Reload, window: &mut Window, cx: &mut Context<Self>) {
        self.save(&SaveRequest, window, cx);
        let selected_path = self.selected.map(|ix| self.entries[ix].path.clone());

        let (entries, errors) = collection::load(&self.dir);
        self.entries = entries;
        self.load_errors = errors.iter().map(|e| format!("{e:#}").into()).collect();

        match Environments::load(&self.dir) {
            Ok(envs) => self.envs = envs,
            Err(err) => {
                self.envs = Environments::default();
                self.load_errors.push(format!("{err:#}").into());
            }
        }
        let names: Vec<SharedString> = self.envs.names().map(|n| n.to_string().into()).collect();
        let current_env = self.env_select.read(cx).selected_value().cloned();
        self.env_select.update(cx, |state, cx| {
            let has_envs = !names.is_empty();
            state.set_items(names, window, cx);
            match current_env {
                Some(env) => state.set_selected_value(&env, window, cx),
                None if has_envs => {
                    state.set_selected_index(Some(IndexPath::default()), window, cx)
                }
                None => state.set_selected_index(None, window, cx),
            }
        });

        let selected = selected_path
            .and_then(|path| self.entries.iter().position(|e| e.path == path))
            .or(if self.entries.is_empty() {
                None
            } else {
                Some(0)
            });
        self.selected = None;
        if let Some(ix) = selected {
            self.select(ix, window, cx);
        }
        cx.notify();
    }

    fn select(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected == Some(ix) {
            return;
        }
        self.save(&SaveRequest, window, cx);
        self.selected = Some(ix);
        self.outcome = None;
        self.in_flight = None;

        let request = self.entries[ix].request.clone();
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
        cx.notify();
    }

    /// The request as currently shown in the editors.
    fn edited_request(&self, ix: usize, cx: &App) -> Request {
        Request {
            name: self.entries[ix].request.name.clone(),
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
        let Some(ix) = self.selected else { return };
        if !self.dirty {
            return;
        }
        let request = self.edited_request(ix, cx);
        let entry = &mut self.entries[ix];
        match request.save(&entry.path) {
            Ok(()) => {
                entry.request = request;
                self.dirty = false;
            }
            Err(err) => self.show_error(format!("{err:#}"), window, cx),
        }
        cx.notify();
    }

    fn new_request(&mut self, _: &NewRequest, window: &mut Window, cx: &mut Context<Self>) {
        match collection::create(&self.dir) {
            Ok(entry) => {
                self.save(&SaveRequest, window, cx);
                self.entries.push(entry);
                self.select(self.entries.len() - 1, window, cx);
                let path = self.entries[self.entries.len() - 1].path.clone();
                self.start_rename(&path, window, cx);
            }
            Err(err) => self.show_error(format!("{err:#}"), window, cx),
        }
    }

    fn delete_selected(&mut self, _: &DeleteRequest, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.selected {
            let path = self.entries[ix].path.clone();
            self.confirm_delete(&path, window, cx);
        }
    }

    fn rename_selected(&mut self, _: &RenameRequest, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.selected {
            let path = self.entries[ix].path.clone();
            self.start_rename(&path, window, cx);
        }
    }

    /// Replaces the entry's name in the sidebar with an input.
    /// Enter or leaving the input commits, Escape cancels.
    fn start_rename(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.entries.iter().find(|e| e.path == path) else {
            return;
        };
        let name = entry.request.name.clone();
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
            path: path.to_path_buf(),
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
        let Some(ix) = self.entries.iter().position(|e| e.path == renaming.path) else {
            return;
        };
        if !commit || name.is_empty() || name == self.entries[ix].request.name {
            return;
        }
        // Keep unsaved edits: the rename rewrites the file from the saved request.
        if self.selected == Some(ix) {
            self.save(&SaveRequest, window, cx);
        }
        match collection::rename(&self.entries[ix], &name) {
            Ok(entry) => self.entries[ix] = entry,
            Err(err) => self.show_error(format!("{err:#}"), window, cx),
        }
    }

    /// Asks for confirmation, then deletes the request's file.
    fn confirm_delete(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.entries.iter().find(|e| e.path == path) else {
            return;
        };
        let title: SharedString = format!("Delete “{}”?", entry.request.name).into();
        let file: SharedString = format!(
            "{} will be removed from disk.",
            path.strip_prefix(&self.dir).unwrap_or(path).display()
        )
        .into();
        let this = cx.entity().downgrade();
        let path = path.to_path_buf();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let this = this.clone();
            let path = path.clone();
            alert
                .title(title.clone())
                .description(file.clone())
                // `button_props` replaces all button settings, so `show_cancel` goes after it.
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Delete")
                        .ok_variant(ButtonVariant::Danger),
                )
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    this.update(cx, |this, cx| this.delete(&path, window, cx))
                        .ok();
                    true
                })
        });
    }

    fn delete(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.entries.iter().position(|e| e.path == path) else {
            return;
        };
        if let Err(err) = collection::delete(&self.entries[ix]) {
            self.show_error(format!("{err:#}"), window, cx);
            return;
        }
        self.entries.remove(ix);
        match self.selected {
            Some(selected) if selected == ix => {
                // The selected request is gone: its unsaved edits go with it.
                self.selected = None;
                self.dirty = false;
                if !self.entries.is_empty() {
                    self.select(ix.min(self.entries.len() - 1), window, cx);
                }
            }
            Some(selected) if selected > ix => self.selected = Some(selected - 1),
            _ => {}
        }
        cx.notify();
    }

    fn variables(&self, cx: &App) -> Variables {
        self.env_select
            .read(cx)
            .selected_value()
            .and_then(|name| self.envs.variables(name))
            .cloned()
            .unwrap_or_default()
    }

    fn send(&mut self, _: &SendRequest, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.selected else { return };
        self.save(&SaveRequest, window, cx);
        let request = self.edited_request(ix, cx);
        let vars = self.variables(cx);

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
        self.theme_settings.toggle_mode(window, cx);
        self.sync_theme_select(window, cx);
        cx.notify();
    }

    /// Lists the themes of the current mode in the picker, with the active one selected.
    fn sync_theme_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let names = theme::names_for_current_mode(cx);
        let current = theme::current_name(cx);
        self.theme_select.update(cx, |state, cx| {
            state.set_items(names, window, cx);
            state.set_selected_value(&current, window, cx);
        });
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let dir_name: SharedString = self
            .dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.dir.display().to_string())
            .into();

        v_flex()
            .size_full()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .child(
                h_flex()
                    .h(px(40.))
                    .px_2()
                    .gap_1()
                    .justify_between()
                    .border_b_1()
                    .border_color(theme.sidebar_border)
                    .child(
                        div()
                            .pl_1()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(dir_name),
                    )
                    .child(
                        h_flex()
                            .child(
                                Button::new("toggle-theme")
                                    .ghost()
                                    .small()
                                    .icon(if cx.theme().is_dark() {
                                        IconName::Sun
                                    } else {
                                        IconName::Moon
                                    })
                                    .tooltip("Toggle dark mode (Ctrl+Shift+T)")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_theme(&ToggleTheme, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("reload")
                                    .ghost()
                                    .small()
                                    .icon(IconName::RefreshCw)
                                    .tooltip("Reload from disk (Ctrl+R)")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.reload(&Reload, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("new-request")
                                    .ghost()
                                    .small()
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
                    .id("requests")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_1()
                    .gap_px()
                    .children(
                        self.entries
                            .iter()
                            .enumerate()
                            .map(|(ix, entry)| self.render_entry(ix, entry, cx)),
                    )
                    .when(self.entries.is_empty(), |this| {
                        this.child(
                            div()
                                .p_2()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("No requests yet. Press + to create one."),
                        )
                    }),
            )
            .children(self.load_errors.iter().map(|error| {
                div()
                    .p_2()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(error.clone())
            }))
            .child(
                div()
                    .p_2()
                    .border_t_1()
                    .border_color(cx.theme().sidebar_border)
                    .child(
                        Select::new(&self.theme_select)
                            .small()
                            .icon(IconName::Palette)
                            .accessibility_label("Color theme"),
                    ),
            )
    }

    fn render_entry(&self, ix: usize, entry: &Entry, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let selected = self.selected == Some(ix);
        let request = &entry.request;
        let name: SharedString = if request.name.is_empty() {
            "Untitled".into()
        } else {
            request.name.clone().into()
        };
        let id = SharedString::from(entry.path.to_string_lossy().into_owned());

        h_flex()
            .id(id)
            .gap_2()
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .text_sm()
            .cursor_pointer()
            .when(selected, |this| this.bg(theme.list_active))
            .when(!selected, |this| {
                this.hover(|this| this.bg(theme.list_hover))
            })
            .child(
                div()
                    .w(px(52.))
                    .flex_none()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(method_color(&request.method, cx))
                    .child(SharedString::from(request.method.clone())),
            )
            .map(|this| match &self.renaming {
                Some(renaming) if renaming.path == entry.path => this
                    .child(div().flex_1().child(Input::new(&renaming.input).xsmall()))
                    .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "escape" {
                            cx.stop_propagation();
                            this.finish_rename(false, window, cx);
                        }
                    })),
                _ => this.child(div().truncate().child(name)),
            })
            .when(selected && self.dirty, |this| {
                this.child(div().text_color(cx.theme().muted_foreground).child("•"))
            })
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.select(ix, window, cx);
                this.focus_handle.focus(window, cx);
                if event.click_count() == 2 {
                    let path = this.entries[ix].path.clone();
                    this.start_rename(&path, window, cx);
                }
            }))
            .context_menu({
                let this = cx.entity().downgrade();
                let path = entry.path.clone();
                move |menu, _, _| {
                    let (rename_this, rename_path) = (this.clone(), path.clone());
                    let (delete_this, delete_path) = (this.clone(), path.clone());
                    menu.item(
                        PopupMenuItem::new("Rename")
                            .icon(IconName::Pencil)
                            .on_click(move |_, window, cx| {
                                rename_this
                                    .update(cx, |this, cx| {
                                        this.start_rename(&rename_path, window, cx)
                                    })
                                    .ok();
                            }),
                    )
                    .item(
                        PopupMenuItem::new("Delete").icon(IconName::Trash).on_click(
                            move |_, window, cx| {
                                delete_this
                                    .update(cx, |this, cx| {
                                        this.confirm_delete(&delete_path, window, cx)
                                    })
                                    .ok();
                            },
                        ),
                    )
                }
            })
            .into_any_element()
    }

    fn render_url_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let sending = self.in_flight.is_some();
        h_flex()
            .gap_2()
            .p_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .w(px(120.))
                    .flex_none()
                    .child(Select::new(&self.method)),
            )
            .child(div().flex_1().child(Input::new(&self.url)))
            .when(
                !self.env_select.read(cx).selected_value().is_none(),
                |this| {
                    this.child(
                        div()
                            .w(px(130.))
                            .flex_none()
                            .child(Select::new(&self.env_select).placeholder("No env")),
                    )
                },
            )
            .child(
                Button::new("send")
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
                h_flex().gap_2().px_2().justify_between().child(
                    TabBar::new("request-tabs")
                        .underline()
                        .small()
                        .selected_index(self.request_tab)
                        .on_click(cx.listener(|this, ix: &usize, _, cx| {
                            this.request_tab = *ix;
                            cx.notify();
                        }))
                        .child(Tab::new().label("Body"))
                        .child(Tab::new().label("Headers")),
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
                            .child(Tab::new().label("Response"))
                            .child(Tab::new().label("Headers")),
                    )
                    .child(div().text_sm().min_w_0().child(summary)),
            )
            .child(div().flex_1().min_h_0().child(content))
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
                                    .flex_none()
                                    .child(self.render_request(cx)),
                            )
                            .child(self.render_response(cx).into_any_element()),
                    ),
                )
                .into_any_element()
        } else {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .text_color(cx.theme().muted_foreground)
                .child("No request selected")
                .child(
                    Button::new("empty-new")
                        .icon(IconName::Plus)
                        .label("New request")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.new_request(&NewRequest, window, cx)
                        })),
                )
                .into_any_element()
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
            .on_action(cx.listener(Self::rename_selected))
            .on_action(cx.listener(Self::delete_selected))
            .child(
                h_resizable("sidebar-main")
                    .child(
                        resizable_panel()
                            .size(px(240.))
                            .flex_none()
                            .child(self.render_sidebar(cx)),
                    )
                    .child(main),
            )
            // gpui-kit's Root keeps overlays but leaves drawing them to the view.
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}

fn editor(state: &Entity<EditorState>, cx: &App) -> Editor {
    Editor::new(state)
        .bordered(false)
        .h(relative(1.))
        .font_family(cx.theme().mono_font_family.clone())
        .text_size(cx.theme().mono_font_size)
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
