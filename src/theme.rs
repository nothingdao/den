use std::cell::Cell;
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

use ratatui::style::Color;

const CONFIG_DIR_NAME: &str = "den";
// ── Theme ─────────────────────────────────────────────────────────────────────

const THEME_FILE_NAME: &str = "theme.toml";

#[derive(serde::Deserialize)]
struct DenThemeConfig {
    #[serde(default = "den_default_bg")]
    bg: String,
    #[serde(default = "den_default_fg")]
    fg: String,
    #[serde(default = "den_default_accent")]
    accent: String,
    #[serde(default = "den_default_sel_fg")]
    sel_fg: String,
    #[serde(default = "den_default_fg_dim")]
    fg_dim: String,
    #[serde(default = "den_default_fg_xdim")]
    fg_xdim: String,
    #[serde(default = "den_default_border")]
    border: String,
    #[serde(default = "den_default_surface")]
    surface: String,
    #[serde(default = "den_default_green")]
    green: String,
    #[serde(default = "den_default_red")]
    red: String,
    #[serde(default = "den_default_yellow")]
    yellow: String,
}

fn den_default_bg() -> String {
    "#101010".into()
}
fn den_default_fg() -> String {
    "#ffffff".into()
}
fn den_default_accent() -> String {
    "#e8b887".into()
}
fn den_default_sel_fg() -> String {
    "#101010".into()
}
fn den_default_fg_dim() -> String {
    "#A0A0A0".into()
}
fn den_default_fg_xdim() -> String {
    "#7E7E7E".into()
}
fn den_default_border() -> String {
    "#232323".into()
}
fn den_default_surface() -> String {
    "#1C1C1C".into()
}
fn den_default_green() -> String {
    "#90b99f".into()
}
fn den_default_red() -> String {
    "#f5a191".into()
}
fn den_default_yellow() -> String {
    "#e6b99d".into()
}

#[derive(Default, serde::Deserialize)]
struct DenThemeFile {
    #[serde(default)]
    theme: DenThemeConfig,
}

impl Default for DenThemeConfig {
    fn default() -> Self {
        Self {
            bg: den_default_bg(),
            fg: den_default_fg(),
            accent: den_default_accent(),
            sel_fg: den_default_sel_fg(),
            fg_dim: den_default_fg_dim(),
            fg_xdim: den_default_fg_xdim(),
            border: den_default_border(),
            surface: den_default_surface(),
            green: den_default_green(),
            red: den_default_red(),
            yellow: den_default_yellow(),
        }
    }
}

#[derive(Clone, Copy)]
pub struct DenTheme {
    pub bg: Color,
    pub fg: Color,
    pub accent: Color,
    pub sel_fg: Color,
    pub fg_dim: Color,
    pub fg_xdim: Color,
    pub border: Color,
    pub surface: Color,
    pub green: Color,
    pub red: Color,
    pub yellow: Color,
}

thread_local! {
    static DEN_THEME: Cell<Option<DenTheme>> = const { Cell::new(None) };
}

pub fn theme() -> DenTheme {
    DEN_THEME.with(|c| c.get().unwrap_or_else(default_den_theme))
}

fn default_den_theme() -> DenTheme {
    den_theme_from_config(&DenThemeConfig::default())
}

fn den_theme_from_config(cfg: &DenThemeConfig) -> DenTheme {
    DenTheme {
        bg: den_hex_color(&cfg.bg),
        fg: den_hex_color(&cfg.fg),
        accent: den_hex_color(&cfg.accent),
        sel_fg: den_hex_color(&cfg.sel_fg),
        fg_dim: den_hex_color(&cfg.fg_dim),
        fg_xdim: den_hex_color(&cfg.fg_xdim),
        border: den_hex_color(&cfg.border),
        surface: den_hex_color(&cfg.surface),
        green: den_hex_color(&cfg.green),
        red: den_hex_color(&cfg.red),
        yellow: den_hex_color(&cfg.yellow),
    }
}

fn den_hex_color(hex: &str) -> Color {
    let h = hex.trim_start_matches('#');
    let r = u8::from_str_radix(&h[0..2], 16).unwrap_or(255);
    let g = u8::from_str_radix(&h[2..4], 16).unwrap_or(255);
    let b = u8::from_str_radix(&h[4..6], 16).unwrap_or(255);
    Color::Rgb(r, g, b)
}

fn den_theme_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(CONFIG_DIR_NAME).join(THEME_FILE_NAME))
}

pub fn init_den_theme() {
    if let Some(path) = den_theme_path()
        && let Ok(s) = fs::read_to_string(&path)
        && let Ok(file) = toml::from_str::<DenThemeFile>(&s)
    {
        DEN_THEME.with(|c| c.set(Some(den_theme_from_config(&file.theme))));
        return;
    }
    DEN_THEME.with(|c| c.set(Some(default_den_theme())));
}

pub fn reload_den_theme_if_changed(mtime: &mut Option<SystemTime>) {
    let Some(path) = den_theme_path() else { return };
    let Ok(meta) = fs::metadata(&path) else {
        return;
    };
    let Ok(modified) = meta.modified() else {
        return;
    };
    if mtime.is_none_or(|last| modified > last) {
        *mtime = Some(modified);
        if let Ok(s) = fs::read_to_string(&path)
            && let Ok(file) = toml::from_str::<DenThemeFile>(&s)
        {
            DEN_THEME.with(|c| c.set(Some(den_theme_from_config(&file.theme))));
        }
    }
}
