#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    DefaultFg,
    DefaultBg,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

// KDE Konsole Breeze 16-color ANSI palette (RGBA8)
const ANSI_16: [u32; 16] = [
    0x2326_29FF, // 0 Black
    0xED15_15FF, // 1 Red
    0x11D1_16FF, // 2 Green
    0xF674_00FF, // 3 Yellow
    0x1D99_F3FF, // 4 Blue
    0x9B59_B6FF, // 5 Magenta
    0x1ABC_9CFF, // 6 Cyan
    0xFCFC_FCFF, // 7 White
    0x7F8C_8DFF, // 8 Bright Black
    0xC039_2BFF, // 9 Bright Red
    0x1CDC_9AFF, // 10 Bright Green
    0xFDBC_4BFF, // 11 Bright Yellow
    0x3DAE_E9FF, // 12 Bright Blue
    0x8E44_ADFF, // 13 Bright Magenta
    0x16A0_85FF, // 14 Bright Cyan
    0xFFFF_FFFF, // 15 Bright White
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
