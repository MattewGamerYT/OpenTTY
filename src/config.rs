use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub app_id: String,
    pub title: String,
    pub shell: String,
    pub font_family_preferences: Vec<String>,
    pub font_size_pt: f32,
    pub initial_cols: u16,
    pub initial_rows: u16,
    pub padding_px: f32,
    pub scrollback_lines: usize,
    pub default_fg: u32,
    pub default_bg: u32,
    pub cursor_color: u32,
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
            font_size_pt: 11.5,
            initial_cols: 110,
            initial_rows: 32,
            padding_px: 10.0,
            scrollback_lines: 10_000,
            // Breeze / Kanagawa-inspired crisp dark palette (RGBA8)
            default_fg: 0xDCD7_BAFF,
            default_bg: 0x1616_1DFF,
            cursor_color: 0x7E9C_D8FF,
        }
    }
}
