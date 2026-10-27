//! Color themes and light/dark mode.
//!
//! The mode follows the system until the user toggles it. The picked mode and
//! the theme chosen for each mode are remembered in
//! `$XDG_CONFIG_HOME/reqo/settings.toml`.

use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::{ActiveTheme, Theme, ThemeConfig, ThemeMode, ThemeRegistry};
use gpui_kit::*;
use serde::{Deserialize, Serialize};

/// Dark theme used until the user picks one. Light mode keeps gpui-kit's default.
const DEFAULT_DARK: &str = "Tokyo Night";

const BUNDLED: &[&str] = &[
    include_str!("../themes/tokyonight.json"),
    include_str!("../themes/catppuccin.json"),
    include_str!("../themes/ayu.json"),
    include_str!("../themes/gruvbox.json"),
    include_str!("../themes/everforest.json"),
];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThemeSettings {
    /// `None` follows the system appearance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mode: Option<Mode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dark_theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    light_theme: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    Light,
    Dark,
}

impl From<Mode> for ThemeMode {
    fn from(mode: Mode) -> Self {
        match mode {
            Mode::Light => ThemeMode::Light,
            Mode::Dark => ThemeMode::Dark,
        }
    }
}

fn settings_path() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("reqo").join("settings.toml"))
}

impl ThemeSettings {
    pub fn load() -> Self {
        settings_path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        let Some(path) = settings_path() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = toml::to_string(self) {
            let _ = std::fs::write(path, text);
        }
    }

    pub fn follows_system(&self) -> bool {
        self.mode.is_none()
    }

    fn theme_name(&self, mode: ThemeMode) -> Option<&str> {
        match mode {
            ThemeMode::Dark => Some(self.dark_theme.as_deref().unwrap_or(DEFAULT_DARK)),
            ThemeMode::Light => self.light_theme.as_deref(),
        }
    }

    /// Applies the settings: the chosen theme for each mode, then the mode.
    pub fn apply(&self, window: &mut Window, cx: &mut App) {
        let dark = self
            .theme_name(ThemeMode::Dark)
            .and_then(|name| find_theme(name, cx));
        let light = self
            .theme_name(ThemeMode::Light)
            .and_then(|name| find_theme(name, cx));
        let theme = Theme::global_mut(cx);
        if let Some(dark) = dark {
            theme.dark_theme = dark;
        }
        if let Some(light) = light {
            theme.light_theme = light;
        }
        let mode = self
            .mode
            .map(ThemeMode::from)
            .unwrap_or_else(|| window.appearance().into());
        Theme::change(mode, Some(window), cx);
    }

    /// Switches to the opposite mode and remembers it.
    pub fn toggle_mode(&mut self, window: &mut Window, cx: &mut App) {
        self.mode = Some(if cx.theme().is_dark() {
            Mode::Light
        } else {
            Mode::Dark
        });
        self.save();
        self.apply(window, cx);
    }

    /// The mode as offered in settings: one of [`MODE_CHOICES`].
    pub fn mode_choice(&self) -> &'static str {
        match self.mode {
            None => MODE_CHOICES[0],
            Some(Mode::Light) => MODE_CHOICES[1],
            Some(Mode::Dark) => MODE_CHOICES[2],
        }
    }

    /// Sets the mode from one of [`MODE_CHOICES`] and remembers it.
    pub fn set_mode_choice(&mut self, choice: &str, window: &mut Window, cx: &mut App) {
        self.mode = match choice {
            "Light" => Some(Mode::Light),
            "Dark" => Some(Mode::Dark),
            _ => None,
        };
        self.save();
        self.apply(window, cx);
    }

    /// Uses `name` for dark or light mode and remembers it.
    pub fn set_theme(&mut self, dark: bool, name: &str, window: &mut Window, cx: &mut App) {
        let name = Some(name.to_string());
        if dark {
            self.dark_theme = name;
        } else {
            self.light_theme = name;
        }
        self.save();
        self.apply(window, cx);
    }
}

/// Mode options in settings; the first one follows the system.
pub const MODE_CHOICES: [&str; 3] = ["System", "Light", "Dark"];

/// Registers the bundled themes. Call once at startup.
pub fn init(cx: &mut App) {
    let registry = ThemeRegistry::global_mut(cx);
    for json in BUNDLED {
        if let Err(err) = registry.load_themes_from_str(json) {
            eprintln!("reqo: bundled theme failed to load: {err:#}");
        }
    }
}

fn find_theme(name: &str, cx: &App) -> Option<Rc<ThemeConfig>> {
    ThemeRegistry::global(cx).themes().get(name).cloned()
}

/// Names of the dark or light themes, sorted.
pub fn names(dark: bool, cx: &App) -> Vec<SharedString> {
    let mut names: Vec<SharedString> = ThemeRegistry::global(cx)
        .themes()
        .values()
        .filter(|theme| theme.mode.is_dark() == dark)
        .map(|theme| theme.name.clone())
        .collect();
    names.sort();
    names
}

/// Name of the theme used in dark or light mode.
pub fn name(dark: bool, cx: &App) -> SharedString {
    let theme = Theme::global(cx);
    if dark {
        theme.dark_theme.name.clone()
    } else {
        theme.light_theme.name.clone()
    }
}
