//! The environments editor: the environments of the workspace or of one
//! collection, written to `reqo.env.toml` as they are edited.

use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Disableable as _, IndexPath, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariant, ButtonVariants as _},
    dialog::DialogButtonProps,
    h_flex,
    input::{Input, InputEvent, InputState},
    select::{Select, SelectEvent, SelectState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use reqo_core::{ENV_FILE, Environments, Variables};

/// A place environments can be edited in: the workspace or a collection.
pub struct Scope {
    pub label: SharedString,
    pub dir: PathBuf,
    /// Shown under the scope picker.
    pub hint: SharedString,
}

pub enum EnvEditorEvent {
    /// The environments file in this folder was written.
    Changed(PathBuf),
}

/// One variable: its name and value inputs.
struct VarRow {
    key: Entity<InputState>,
    value: Entity<InputState>,
    _subscriptions: [Subscription; 2],
}

pub struct EnvEditor {
    scopes: Vec<Scope>,
    scope: usize,
    scope_select: Entity<SelectState<Vec<SharedString>>>,
    envs: Environments,
    /// Set when the file could not be read; editing is off so it is not overwritten.
    load_error: Option<SharedString>,
    selected: Option<String>,
    name: Entity<InputState>,
    rows: Vec<VarRow>,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<EnvEditorEvent> for EnvEditor {}

impl EnvEditor {
    pub fn new(
        scopes: Vec<Scope>,
        scope: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let labels: Vec<SharedString> = scopes.iter().map(|s| s.label.clone()).collect();
        let scope_select =
            cx.new(|cx| SelectState::new(labels, Some(IndexPath::new(scope)), window, cx));
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Environment name"));
        let subscriptions = vec![
            cx.subscribe_in(
                &scope_select,
                window,
                |this, _, event: &SelectEvent<_>, window, cx| {
                    if let SelectEvent::Confirm(Some(label)) = event
                        && let Some(ix) = this.scopes.iter().position(|s| s.label == *label)
                    {
                        this.open_scope(ix, window, cx);
                    }
                },
            ),
            cx.subscribe_in(&name, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.rename_selected(cx);
                }
            }),
        ];

        let mut this = Self {
            scopes,
            scope,
            scope_select,
            envs: Environments::default(),
            load_error: None,
            selected: None,
            name,
            rows: Vec::new(),
            error: None,
            _subscriptions: subscriptions,
        };
        this.open_scope(scope, window, cx);
        this
    }

    fn dir(&self) -> &PathBuf {
        &self.scopes[self.scope].dir
    }

    fn open_scope(&mut self, scope: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.scope = scope;
        self.error = None;
        match Environments::load(self.dir()) {
            Ok(envs) => {
                self.envs = envs;
                self.load_error = None;
            }
            Err(err) => {
                self.envs = Environments::default();
                self.load_error = Some(format!("{err:#}").into());
            }
        }
        let first = self.envs.names().next().map(str::to_string);
        self.select_env(first, window, cx);
    }

    fn select_env(&mut self, name: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = name.clone();
        self.error = None;
        // Set before the input so its change event sees no rename.
        self.name.update(cx, |state, cx| {
            state.set_value(name.clone().unwrap_or_default(), window, cx)
        });
        let vars = name
            .and_then(|name| self.envs.variables(&name).cloned())
            .unwrap_or_default();
        self.rows = vars
            .iter()
            .map(|(key, value)| self.new_row(key, value, window, cx))
            .collect();
        cx.notify();
    }

    fn new_row(
        &self,
        key: &str,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> VarRow {
        let key = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("name")
                .default_value(key.to_string())
        });
        let value = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("value")
                .default_value(value.to_string())
        });
        let on_change = |this: &mut Self, _, event: &InputEvent, cx: &mut Context<Self>| {
            if matches!(event, InputEvent::Change) {
                this.store_variables(cx);
            }
        };
        VarRow {
            _subscriptions: [
                cx.subscribe(&key, on_change),
                cx.subscribe(&value, on_change),
            ],
            key,
            value,
        }
    }

    /// Writes the variables shown for the selected environment. Rows without a name are skipped.
    fn store_variables(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.selected.clone() else {
            return;
        };
        let vars: Variables = self
            .rows
            .iter()
            .filter_map(|row| {
                let key = row.key.read(cx).value().trim().to_string();
                let value = row.value.read(cx).value().to_string();
                (!key.is_empty()).then_some((key, value))
            })
            .collect();
        self.envs.set_variables(&name, vars);
        self.save(cx);
    }

    fn rename_selected(&mut self, cx: &mut Context<Self>) {
        let Some(old) = self.selected.clone() else {
            return;
        };
        let new = self.name.read(cx).value().trim().to_string();
        if new == old {
            self.error = None;
            cx.notify();
            return;
        }
        match self.envs.rename(&old, &new) {
            Ok(()) => {
                self.selected = Some(new);
                self.error = None;
                self.save(cx);
            }
            Err(err) => {
                self.error = Some(format!("{err:#}").into());
                cx.notify();
            }
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.load_error.is_some() {
            return;
        }
        match self.envs.save(self.dir()) {
            Ok(()) => cx.emit(EnvEditorEvent::Changed(self.dir().clone())),
            Err(err) => self.error = Some(format!("{err:#}").into()),
        }
        cx.notify();
    }

    fn add_env(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.envs.add("new");
        self.save(cx);
        self.select_env(Some(name), window, cx);
        self.name.update(cx, |state, cx| {
            state.focus(window, cx);
            state.select_all(window, cx);
        });
    }

    fn confirm_delete_env(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.selected.clone() else {
            return;
        };
        let count = self.rows.len();
        let this = cx.entity().downgrade();
        let title = SharedString::from(format!("Delete environment “{name}”?"));
        let description = SharedString::from(match count {
            1 => "Its variable will be deleted.".to_string(),
            n => format!("Its {n} variables will be deleted."),
        });
        window.open_alert_dialog(cx, move |alert, _, _| {
            let this = this.clone();
            let name = name.clone();
            alert
                .title(title.clone())
                .description(description.clone())
                // `button_props` replaces all button settings, so `show_cancel` goes after it.
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Delete")
                        .ok_variant(ButtonVariant::Danger),
                )
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    this.update(cx, |this, cx| {
                        this.envs.remove(&name);
                        this.save(cx);
                        let first = this.envs.names().next().map(str::to_string);
                        this.select_env(first, window, cx);
                    })
                    .ok();
                    true
                })
        });
    }

    fn add_variable(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let row = self.new_row("", "", window, cx);
        row.key.update(cx, |state, cx| state.focus(window, cx));
        self.rows.push(row);
        cx.notify();
    }

    fn remove_variable(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.rows.remove(ix);
        self.store_variables(cx);
    }

    /// Opens the file in the system editor, writing it first if it does not exist.
    fn open_file(&mut self, cx: &mut Context<Self>) {
        let path = self.dir().join(ENV_FILE);
        if !path.exists() {
            self.save(cx);
        }
        cx.open_with_system(&path);
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .w(px(190.))
            .flex_none()
            .border_r_1()
            .border_color(theme.border)
            .child(
                v_flex()
                    .id("env-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_1()
                    .gap_px()
                    .children(self.envs.names().enumerate().map(|(ix, name)| {
                        let selected = self.selected.as_deref() == Some(name);
                        let name = name.to_string();
                        div()
                            .id(("env", ix))
                            .px_2()
                            .py_1()
                            .rounded(theme.radius)
                            .text_sm()
                            .truncate()
                            .cursor_pointer()
                            .when(selected, |this| this.bg(theme.list_active))
                            .when(!selected, |this| {
                                this.hover(|this| this.bg(theme.list_hover))
                            })
                            .child(name.clone())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_env(Some(name.clone()), window, cx)
                            }))
                    })),
            )
            .child(
                div().p_1().border_t_1().border_color(theme.border).child(
                    Button::new("add-env")
                        .when(self.load_error.is_none(), |this| this.cursor_pointer())
                        .ghost()
                        .small()
                        .w_full()
                        .icon(IconName::Plus)
                        .label("Add environment")
                        .disabled(self.load_error.is_some())
                        .on_click(cx.listener(|this, _, window, cx| this.add_env(window, cx))),
                ),
            )
    }

    fn render_variables(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        if self.selected.is_none() {
            return v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(if self.load_error.is_some() {
                    "Fix the file to edit it here."
                } else {
                    "No environments yet."
                })
                .into_any_element();
        }
        let label = |text: &'static str| {
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.muted_foreground)
                .child(text)
        };

        v_flex()
            .flex_1()
            .min_w_0()
            .p_3()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.name).small()))
                    .child(
                        Button::new("delete-env")
                            .cursor_pointer()
                            .ghost()
                            .small()
                            .icon(IconName::Trash)
                            .tooltip("Delete environment")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm_delete_env(window, cx)
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .pt_1()
                    .child(div().w(px(170.)).flex_none().child(label("VARIABLE")))
                    .child(div().flex_1().child(label("VALUE"))),
            )
            .child(
                v_flex()
                    .id("env-vars")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .gap_1()
                    .children(self.rows.iter().enumerate().map(|(ix, row)| {
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(170.))
                                    .flex_none()
                                    .child(Input::new(&row.key).small()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(Input::new(&row.value).small()),
                            )
                            .child(
                                Button::new(("remove-var", ix))
                                    .cursor_pointer()
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Trash)
                                    .tooltip("Remove variable")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.remove_variable(ix, cx)
                                    })),
                            )
                    }))
                    .child(
                        h_flex().child(
                            Button::new("add-var")
                                .cursor_pointer()
                                .ghost()
                                .small()
                                .icon(IconName::Plus)
                                .label("Add variable")
                                .on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.add_variable(window, cx)
                                    }),
                                ),
                        ),
                    ),
            )
            .into_any_element()
    }
}

impl Render for EnvEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let hint = self.scopes[self.scope].hint.clone();
        let error = self.load_error.clone().or_else(|| self.error.clone());

        v_flex()
            .gap_2()
            .h(px(440.))
            .child(
                h_flex()
                    .gap_2()
                    .justify_between()
                    .child(
                        div()
                            .w(px(300.))
                            .child(Select::new(&self.scope_select).cursor_pointer().small()),
                    )
                    .child(
                        Button::new("open-env-file")
                            .cursor_pointer()
                            .ghost()
                            .small()
                            .icon(IconName::FileText)
                            .label("Open file")
                            .tooltip(SharedString::from(
                                self.dir().join(ENV_FILE).display().to_string(),
                            ))
                            .on_click(cx.listener(|this, _, _, cx| this.open_file(cx))),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(hint),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .border_1()
                    .border_color(theme.border)
                    .rounded(theme.radius)
                    .child(self.render_list(cx))
                    .child(self.render_variables(cx)),
            )
            .when_some(error, |this, error| {
                this.child(div().text_xs().text_color(cx.theme().danger).child(error))
            })
    }
}
