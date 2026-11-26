use std::fs;
use std::path::PathBuf;

use crate::terminal::grid::CursorStyle;

#[derive(Debug, Clone)]
pub struct Config {
    pub app_id: String,
    pub title: String,
    pub shell: String,
    pub font_family_preferences: Vec<String>,
    pub font_size_pt: f32,
    pub auto_zoom: bool,
    pub auto_zoom_min_pt: f32,
    pub auto_zoom_max_pt: f32,
    pub auto_zoom_target_cols: u16,
    pub auto_zoom_target_rows: u16,
    pub padding_px: f32,
    pub initial_cols: u16,
    pub initial_rows: u16,
    pub scrollback_lines: usize,
    pub scroll_lines_per_step: isize,
    pub cursor_style: CursorStyle,
    pub cursor_blink: bool,
    pub cursor_blink_ms: u64,
    pub copy_on_select: bool,
    pub strip_trailing_spaces_on_copy: bool,
    pub bracketed_paste: bool,
    pub shell_quote_dropped_paths: bool,
    pub mouse_reporting: bool,
    pub alternate_scroll: bool,
    pub default_fg: u32,
    pub default_bg: u32,
    pub cursor_color: u32,
    pub selection_fg: u32,
    pub selection_bg: u32,
}

impl Default for Config {
    fn default() -> Self {
        let shell = if PathBuf::from("/usr/bin/fish").exists() {
            "/usr/bin/fish".to_string()
        } else {
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
        };

        Self {
            app_id: "org.opentty.OpenTTY".to_string(),
            title: "OpenTTY".to_string(),
            shell,
            font_family_preferences: vec![
                "JetBrains Mono".to_string(),
                "Fira Code".to_string(),
                "Cascadia Mono".to_string(),
                "Hack".to_string(),
                "DejaVu Sans Mono".to_string(),
                "Noto Sans Mono".to_string(),
                "Liberation Mono".to_string(),
            ],
            font_size_pt: 10.0,
            auto_zoom: true,
            auto_zoom_min_pt: 7.5,
            auto_zoom_max_pt: 10.5,
            auto_zoom_target_cols: 110,
            auto_zoom_target_rows: 32,
            padding_px: 8.0,
            initial_cols: 110,
            initial_rows: 32,
            scrollback_lines: 10_000,
            scroll_lines_per_step: 3,
            cursor_style: CursorStyle::Block,
            cursor_blink: false,
            cursor_blink_ms: 530,
            copy_on_select: false,
            strip_trailing_spaces_on_copy: true,
            bracketed_paste: true,
            shell_quote_dropped_paths: true,
            mouse_reporting: true,
            alternate_scroll: true,
            // KDE Konsole Breeze palette (RGBA8)
            default_fg: 0xFCFC_FCFF,
            default_bg: 0x2326_29FF,
            cursor_color: 0xEFF0_F1FF,
            selection_fg: 0xFCFC_FCFF,
            selection_bg: 0x2D5F_85FF,
        }
    }
}

impl Config {
    /// Loads default configuration and applies overrides from
    /// `$XDG_CONFIG_HOME/opentty/opentty.conf` (or `~/.config/opentty/opentty.conf`) if present.
    pub fn load() -> Self {
        let mut cfg = Self::default();
        let Some(path) = config_file_path() else {
            return cfg;
        };
        let Ok(contents) = fs::read_to_string(&path) else {
            return cfg;
        };

        for (line_no, raw_line) in contents.lines().enumerate() {
            let line = raw_line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let Some((raw_key, raw_val)) = line.split_once('=') else {
                log::warn!("{}:{}: expected key = value", path.display(), line_no + 1);
                continue;
            };
            let key = raw_key.trim();
            let val = raw_val.trim().trim_matches('"').trim_matches('\'');

            match key {
                "shell" if !val.is_empty() => cfg.shell = val.to_string(),
                "title" if !val.is_empty() => cfg.title = val.to_string(),
                "font_family" if !val.is_empty() => {
                    cfg.font_family_preferences.insert(0, val.to_string());
                }
                "font_size_pt" => {
                    if let Ok(v) = val.parse::<f32>() {
                        cfg.font_size_pt = v.clamp(6.0, 48.0);
                    }
                }
                "auto_zoom" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.auto_zoom = b;
                    }
                }
                "auto_zoom_min_pt" => {
                    if let Ok(v) = val.parse::<f32>() {
                        cfg.auto_zoom_min_pt = v.clamp(6.0, 24.0);
                    }
                }
                "auto_zoom_max_pt" => {
                    if let Ok(v) = val.parse::<f32>() {
                        cfg.auto_zoom_max_pt = v.clamp(6.0, 48.0);
                    }
                }
                "auto_zoom_target_cols" => {
                    if let Ok(v) = val.parse::<u16>() {
                        cfg.auto_zoom_target_cols = v.clamp(40, 300);
                    }
                }
                "auto_zoom_target_rows" => {
                    if let Ok(v) = val.parse::<u16>() {
                        cfg.auto_zoom_target_rows = v.clamp(12, 120);
                    }
                }
                "padding_px" => {
                    if let Ok(v) = val.parse::<f32>() {
                        cfg.padding_px = v.clamp(0.0, 64.0);
                    }
                }
                "initial_cols" => {
                    if let Ok(v) = val.parse::<u16>() {
                        cfg.initial_cols = v.clamp(10, 512);
                    }
                }
                "initial_rows" => {
                    if let Ok(v) = val.parse::<u16>() {
                        cfg.initial_rows = v.clamp(4, 256);
                    }
                }
                "scrollback_lines" => {
                    if let Ok(v) = val.parse::<usize>() {
                        cfg.scrollback_lines = v.min(200_000);
                    }
                }
                "scroll_lines_per_step" => {
                    if let Ok(v) = val.parse::<isize>() {
                        cfg.scroll_lines_per_step = v.clamp(1, 50);
                    }
                }
                "cursor_style" => match val.to_ascii_lowercase().as_str() {
                    "block" => cfg.cursor_style = CursorStyle::Block,
                    "beam" | "bar" => cfg.cursor_style = CursorStyle::Beam,
                    "underline" => cfg.cursor_style = CursorStyle::Underline,
                    _ => log::warn!("{}:{}: unknown cursor_style '{val}'", path.display(), line_no + 1),
                },
                "cursor_blink" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.cursor_blink = b;
                    }
                }
                "cursor_blink_ms" => {
                    if let Ok(v) = val.parse::<u64>() {
                        cfg.cursor_blink_ms = v.clamp(150, 2000);
                    }
                }
                "copy_on_select" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.copy_on_select = b;
                    }
                }
                "strip_trailing_spaces_on_copy" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.strip_trailing_spaces_on_copy = b;
                    }
                }
                "bracketed_paste" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.bracketed_paste = b;
                    }
                }
                "shell_quote_dropped_paths" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.shell_quote_dropped_paths = b;
                    }
                }
                "mouse_reporting" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.mouse_reporting = b;
                    }
                }
                "alternate_scroll" => {
                    if let Some(b) = parse_bool(val) {
                        cfg.alternate_scroll = b;
                    }
                }
                "default_fg" => {
                    if let Some(c) = parse_hex_rgba(val) {
                        cfg.default_fg = c;
                    }
                }
                "default_bg" => {
                    if let Some(c) = parse_hex_rgba(val) {
                        cfg.default_bg = c;
                    }
                }
                "cursor_color" => {
                    if let Some(c) = parse_hex_rgba(val) {
                        cfg.cursor_color = c;
                    }
                }
                "selection_fg" => {
                    if let Some(c) = parse_hex_rgba(val) {
                        cfg.selection_fg = c;
                    }
                }
                "selection_bg" => {
                    if let Some(c) = parse_hex_rgba(val) {
                        cfg.selection_bg = c;
                    }
                }
                _ => {
                    log::warn!("{}:{}: unknown config key '{key}'", path.display(), line_no + 1);
                }
            }
        }

        cfg
    }
}

fn config_file_path() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(xdg).join("opentty").join("opentty.conf"));
    }
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(|home| PathBuf::from(home).join(".config").join("opentty").join("opentty.conf"))
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn parse_hex_rgba(s: &str) -> Option<u32> {
    let hex = s.trim_start_matches('#').trim_start_matches("0x");
    match hex.len() {
        6 => u32::from_str_radix(hex, 16).ok().map(|rgb| (rgb << 8) | 0xFF),
        8 => u32::from_str_radix(hex, 16).ok(),
        _ => None,
    }
}
