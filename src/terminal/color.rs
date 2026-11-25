#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    DefaultFg,
    DefaultBg,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

// High-contrast dark-mode 16-color ANSI palette tuned for Fish & TUI legibility (RGBA8)
const ANSI_16: [u32; 16] = [
    0x1E22_2DFF, // 0 Black
    0xF871_71FF, // 1 Red
    0x4ADE_80FF, // 2 Green
    0xFACC_15FF, // 3 Yellow
    0x60A5_FAFF, // 4 Blue
    0xC084_FCFF, // 5 Magenta
    0x22D3_EEFF, // 6 Cyan
    0xCBD5_E1FF, // 7 White
    0x6474_8BFF, // 8 Bright Black (Fish autosuggestions)
    0xFCA5_A5FF, // 9 Bright Red
    0x86EF_ACFF, // 10 Bright Green
    0xFDE0_47FF, // 11 Bright Yellow
    0x93C5_FDFF, // 12 Bright Blue
    0xD8B4_FEFF, // 13 Bright Magenta
    0x67E8_F9FF, // 14 Bright Cyan
    0xF8FA_FCFF, // 15 Bright White
];

pub fn resolve_rgba(color: Color, default_fg: u32, default_bg: u32) -> u32 {
    match color {
        Color::DefaultFg => default_fg,
        Color::DefaultBg => default_bg,
        Color::Rgb(r, g, b) => ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xFF,
        Color::Indexed(idx) => indexed_to_rgba(idx),
    }
}

pub fn indexed_to_rgba(idx: u8) -> u32 {
    if (idx as usize) < 16 {
        return ANSI_16[idx as usize];
    }

    if idx < 232 {
        let n = idx - 16;
        let r = n / 36;
        let g = (n % 36) / 6;
        let b = n % 6;
        let to_u8 = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
        let rr = to_u8(r) as u32;
        let gg = to_u8(g) as u32;
        let bb = to_u8(b) as u32;
        return (rr << 24) | (gg << 16) | (bb << 8) | 0xFF;
    }

    let gray = (8 + (idx - 232) * 10) as u32;
    (gray << 24) | (gray << 16) | (gray << 8) | 0xFF
}
