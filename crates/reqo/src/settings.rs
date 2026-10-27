//! The settings dialog: appearance for now.

use gpui_kit::component::{
    ActiveTheme, Sizable as _, h_flex,
    select::{Select, SelectEvent, SelectState},
    v_flex,
};
use gpui_kit::*;

use crate::theme::{self, MODE_CHOICES, ThemeSettings};

type ChoiceSelect = SelectState<Vec<SharedString>>;

/// Owns the theme settings and the controls that edit them.
pub struct Settings {
    theme: ThemeSettings,
    mode: Entity<ChoiceSelect>,
    dark_theme: Entity<ChoiceSelect>,
    light_theme: Entity<ChoiceSelect>,
    _subscriptions: Vec<Subscription>,
}

impl Settings {
    /// Loads the saved settings and applies them to `window`.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let theme = ThemeSettings::load();
        theme.apply(window, cx);

        let modes: Vec<SharedString> = MODE_CHOICES.iter().map(|&m| m.into()).collect();
        let mode = cx.new(|cx| SelectState::new(modes, None, window, cx));
        let dark_theme = cx.new(|cx| SelectState::new(theme::names(true, cx), None, window, cx));
        let light_theme = cx.new(|cx| SelectState::new(theme::names(false, cx), None, window, cx));

        let subscriptions = vec![
            cx.subscribe_in(
                &mode,
                window,
                |this, _, event: &SelectEvent<_>, window, cx| {
                    if let SelectEvent::Confirm(Some(choice)) = event {
                        this.theme.set_mode_choice(choice, window, cx);
                        cx.notify();
                    }
                },
            ),
            cx.subscribe_in(
                &dark_theme,
                window,
                |this, _, event: &SelectEvent<_>, window, cx| {
                    if let SelectEvent::Confirm(Some(name)) = event {
                        this.theme.set_theme(true, name, window, cx);
                        cx.notify();
                    }
                },
            ),
            cx.subscribe_in(
                &light_theme,
                window,
                |this, _, event: &SelectEvent<_>, window, cx| {
                    if let SelectEvent::Confirm(Some(name)) = event {
                        this.theme.set_theme(false, name, window, cx);
                        cx.notify();
                    }
                },
            ),
            cx.observe_window_appearance(window, |this, window, cx| {
                if this.theme.follows_system() {
                    this.theme.apply(window, cx);
                    cx.notify();
                }
            }),
        ];

        let mut this = Self {
            theme,
            mode,
            dark_theme,
            light_theme,
            _subscriptions: subscriptions,
        };
        this.sync(window, cx);
        this
    }

    /// Switches between light and dark and remembers the choice.
    pub fn toggle_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.theme.toggle_mode(window, cx);
        self.sync(window, cx);
        cx.notify();
    }

    /// Shows the current settings in the controls.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = SharedString::from(self.theme.mode_choice());
        self.mode
            .update(cx, |state, cx| state.set_selected_value(&mode, window, cx));
        for (select, dark) in [(&self.dark_theme, true), (&self.light_theme, false)] {
            let name = theme::name(dark, cx);
            select.update(cx, |state, cx| state.set_selected_value(&name, window, cx));
        }
    }
}

impl Render for Settings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_3()
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().muted_foreground)
                    .child("APPEARANCE"),
            )
            .child(row(
                "Mode",
                "System follows the desktop's light or dark setting.",
                Select::new(&self.mode).small(),
                cx,
            ))
            .child(row(
                "Dark theme",
                "Used in dark mode.",
                Select::new(&self.dark_theme).small(),
                cx,
            ))
            .child(row(
                "Light theme",
                "Used in light mode.",
                Select::new(&self.light_theme).small(),
                cx,
            ))
    }
}

/// A setting: title and hint on the left, its control on the right.
fn row(
    title: &'static str,
    hint: &'static str,
    control: impl IntoElement,
    cx: &App,
) -> impl IntoElement {
    h_flex()
        .gap_4()
        .justify_between()
        .child(
            v_flex()
                .min_w_0()
                .child(div().text_sm().child(title))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(hint),
                ),
        )
        .child(div().w(px(200.)).flex_none().child(control))
}
