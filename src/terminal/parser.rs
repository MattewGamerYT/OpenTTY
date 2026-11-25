use vte::{Params, Perform};

use super::Terminal;
use super::color::Color;
use super::grid::{Cell, CellFlags, CursorStyle, SavedCursor};

impl Perform for Terminal {
    fn print(&mut self, ch: char) {
        self.active_grid_mut().put_char(ch);
        self.dirty = true;
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x07 => {
                // BEL: visual/silent bell
            }
            0x08 => {
                // BS
                let grid = self.active_grid_mut();
                grid.cursor.wrap_pending = false;
                grid.cursor.col = grid.cursor.col.saturating_sub(1);
                self.dirty = true;
            }
            0x09 => {
                // HT (8-column tab stops)
                let grid = self.active_grid_mut();
                grid.cursor.wrap_pending = false;
                let next = ((grid.cursor.col / 8) + 1) * 8;
                grid.cursor.col = next.min(grid.cols.saturating_sub(1));
                self.dirty = true;
            }
            0x0A..=0x0C => {
                // LF, VT, FF
                self.active_grid_mut().linefeed();
                self.dirty = true;
            }
            0x0D => {
                // CR
                let grid = self.active_grid_mut();
                grid.cursor.wrap_pending = false;
                grid.cursor.col = 0;
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: char) {}
    fn put(&mut self, _byte: u8) {}
    fn unhook(&mut self) {}

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if params.is_empty() {
            return;
        }
        match params[0] {
            b"0" | b"2" if params.len() >= 2 => {
                if let Ok(title) = std::str::from_utf8(params[1]) {
                    self.pending_title = Some(title.to_string());
                }
            }
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        match (intermediates, byte) {
            ([], b'7') => {
                let grid = self.active_grid_mut();
                grid.saved_cursor = Some(SavedCursor {
                    col: grid.cursor.col,
                    row: grid.cursor.row,
                    fg: grid.active_fg,
                    bg: grid.active_bg,
                    flags: grid.active_flags,
                });
            }
            ([], b'8') => {
                let grid = self.active_grid_mut();
                if let Some(saved) = grid.saved_cursor {
                    grid.cursor.col = saved.col.min(grid.cols.saturating_sub(1));
                    grid.cursor.row = saved.row.min(grid.rows.saturating_sub(1));
                    grid.cursor.wrap_pending = false;
                    grid.active_fg = saved.fg;
                    grid.active_bg = saved.bg;
                    grid.active_flags = saved.flags;
                    self.dirty = true;
                }
            }
            ([], b'D') => {
                self.active_grid_mut().linefeed();
                self.dirty = true;
            }
            ([], b'E') => {
                let grid = self.active_grid_mut();
                grid.cursor.col = 0;
                grid.linefeed();
                self.dirty = true;
            }
            ([], b'M') => {
                self.active_grid_mut().reverse_index();
                self.dirty = true;
            }
            ([], b'c') => {
                self.reset();
            }
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        let private = intermediates.first().copied() == Some(b'?');
        let space = intermediates.first().copied() == Some(b' ');

        let p = |idx: usize, default: usize| -> usize {
            params
                .iter()
                .nth(idx)
                .and_then(|s| s.first().copied())
                .map(|v| if v == 0 { default } else { v as usize })
                .unwrap_or(default)
        };

        let p_raw = |idx: usize, default: u16| -> u16 {
            params
                .iter()
                .nth(idx)
                .and_then(|s| s.first().copied())
                .unwrap_or(default)
        };

        if private {
            match action {
                'h' | 'l' => {
                    let enable = action == 'h';
                    for item in params.iter() {
                        if let Some(&mode) = item.first() {
                            self.set_private_mode(mode, enable);
                        }
                    }
                    self.dirty = true;
                }
                _ => {}
            }
            return;
        }

        if space && action == 'q' {
            let style = match p_raw(0, 0) {
                0..=2 => CursorStyle::Block,
                3..=4 => CursorStyle::Underline,
                5..=6 => CursorStyle::Beam,
                _ => CursorStyle::Block,
            };
            self.active_grid_mut().cursor.style = style;
            self.dirty = true;
            return;
        }

        // Do not let prefixed CSI sequences (e.g. Fish's `CSI > 4;2 m` modifyOtherKeys
        // or Kitty keyboard `CSI = 5 u` / `CSI < u`) fall through into SGR or cursor restore.
        if !intermediates.is_empty() {
            return;
        }

        match action {
            'A' => {
                let n = p(0, 1);
                let grid = self.active_grid_mut();
                grid.cursor.wrap_pending = false;
                grid.cursor.row = grid.cursor.row.saturating_sub(n);
            }
            'B' | 'e' => {
                let n = p(0, 1);
                let grid = self.active_grid_mut();
                grid.cursor.wrap_pending = false;
                grid.cursor.row = (grid.cursor.row + n).min(grid.rows - 1);
            }
            'C' | 'a' => {
                let n = p(0, 1);
                let grid = self.active_grid_mut();
                grid.cursor.wrap_pending = false;
                grid.cursor.col = (grid.cursor.col + n).min(grid.cols - 1);
            }
            'D' => {
                let n = p(0, 1);
                let grid = self.active_grid_mut();
                grid.cursor.wrap_pending = false;
                grid.cursor.col = grid.cursor.col.saturating_sub(n);
            }
            'E' => {
                let n = p(0, 1);
                let grid = self.active_grid_mut();
                grid.cursor.wrap_pending = false;
                grid.cursor.row = (grid.cursor.row + n).min(grid.rows - 1);
                grid.cursor.col = 0;
            }
            'F' => {
                let n = p(0, 1);
                let grid = self.active_grid_mut();
                grid.cursor.wrap_pending = false;
                grid.cursor.row = grid.cursor.row.saturating_sub(n);
                grid.cursor.col = 0;
            }
            'G' | '`' => {
                let col = p(0, 1).saturating_sub(1);
                let grid = self.active_grid_mut();
                grid.cursor.wrap_pending = false;
                grid.cursor.col = col.min(grid.cols - 1);
            }
            'd' => {
                let row = p(0, 1).saturating_sub(1);
                let grid = self.active_grid_mut();
                grid.cursor.wrap_pending = false;
                grid.cursor.row = row.min(grid.rows - 1);
            }
            'H' | 'f' => {
                let row = p(0, 1).saturating_sub(1);
                let col = p(1, 1).saturating_sub(1);
                let grid = self.active_grid_mut();
                grid.cursor.wrap_pending = false;
                grid.cursor.row = row.min(grid.rows - 1);
                grid.cursor.col = col.min(grid.cols - 1);
            }
            'J' => {
                let mode = p_raw(0, 0);
                let grid = self.active_grid_mut();
                let blank = grid.blank_cell();
                let r = grid.cursor.row;
                let c = grid.cursor.col;
                match mode {
                    0 => {
                        for cell in &mut grid.lines[r].cells[c..] {
                            *cell = blank;
                        }
                        for row in &mut grid.lines[(r + 1)..] {
                            row.clear(blank);
                        }
                    }
                    1 => {
                        for row in &mut grid.lines[..r] {
                            row.clear(blank);
                        }
                        for cell in &mut grid.lines[r].cells[..=c.min(grid.cols - 1)] {
                            *cell = blank;
                        }
                    }
                    2 => {
                        for row in &mut grid.lines {
                            row.clear(blank);
                        }
                    }
                    3 => {
                        for row in &mut grid.lines {
                            row.clear(blank);
                        }
                        grid.scrollback.clear();
                        grid.scroll_offset = 0;
                    }
                    _ => {}
                }
            }
            'K' => {
                let mode = p_raw(0, 0);
                let grid = self.active_grid_mut();
                let blank = grid.blank_cell();
                let r = grid.cursor.row;
                let c = grid.cursor.col;
                match mode {
                    0 => {
                        for cell in &mut grid.lines[r].cells[c..] {
                            *cell = blank;
                        }
                    }
                    1 => {
                        for cell in &mut grid.lines[r].cells[..=c.min(grid.cols - 1)] {
                            *cell = blank;
                        }
                    }
                    2 => {
                        grid.lines[r].clear(blank);
                    }
                    _ => {}
                }
            }
            'L' => {
                let count = p(0, 1);
                let grid = self.active_grid_mut();
                let r = grid.cursor.row;
                if r >= grid.scroll_top && r <= grid.scroll_bottom {
                    let prev_top = grid.scroll_top;
                    grid.scroll_top = r;
                    grid.scroll_down_in_region(count);
                    grid.scroll_top = prev_top;
                }
            }
            'M' => {
                let count = p(0, 1);
                let grid = self.active_grid_mut();
                let r = grid.cursor.row;
                if r >= grid.scroll_top && r <= grid.scroll_bottom {
                    let prev_top = grid.scroll_top;
                    grid.scroll_top = r;
                    grid.scroll_up_in_region(count, false);
                    grid.scroll_top = prev_top;
                }
            }
            'P' => {
                let count = p(0, 1);
                let grid = self.active_grid_mut();
                let blank = grid.blank_cell();
                let r = grid.cursor.row;
                let c = grid.cursor.col;
                let cols = grid.cols;
                let shift = count.min(cols - c);
                let row = &mut grid.lines[r].cells;
                for i in c..(cols - shift) {
                    row[i] = row[i + shift];
                }
                for cell in &mut row[(cols - shift)..] {
                    *cell = blank;
                }
            }
            '@' => {
                let count = p(0, 1);
                let grid = self.active_grid_mut();
                let blank = grid.blank_cell();
                let r = grid.cursor.row;
                let c = grid.cursor.col;
                let cols = grid.cols;
                let shift = count.min(cols - c);
                let row = &mut grid.lines[r].cells;
                for i in ((c + shift)..cols).rev() {
                    row[i] = row[i - shift];
                }
                for cell in &mut row[c..(c + shift)] {
                    *cell = blank;
                }
            }
            'X' => {
                let count = p(0, 1);
                let grid = self.active_grid_mut();
                let blank = grid.blank_cell();
                let r = grid.cursor.row;
                let c = grid.cursor.col;
                let end = (c + count).min(grid.cols);
                for cell in &mut grid.lines[r].cells[c..end] {
                    *cell = blank;
                }
            }
            'S' => {
                let count = p(0, 1);
                self.active_grid_mut().scroll_up_in_region(count, true);
            }
            'T' => {
                let count = p(0, 1);
                self.active_grid_mut().scroll_down_in_region(count);
            }
            'r' => {
                let top = p(0, 1).saturating_sub(1);
                let grid = self.active_grid_mut();
                let bottom = p(1, grid.rows).saturating_sub(1).min(grid.rows - 1);
                if top < bottom {
                    grid.scroll_top = top;
                    grid.scroll_bottom = bottom;
                } else {
                    grid.scroll_top = 0;
                    grid.scroll_bottom = grid.rows - 1;
                }
                grid.cursor.row = 0;
                grid.cursor.col = 0;
                grid.cursor.wrap_pending = false;
            }
            's' => {
                let grid = self.active_grid_mut();
                grid.saved_cursor = Some(SavedCursor {
                    col: grid.cursor.col,
                    row: grid.cursor.row,
                    fg: grid.active_fg,
                    bg: grid.active_bg,
                    flags: grid.active_flags,
                });
            }
            'u' => {
                let grid = self.active_grid_mut();
                if let Some(saved) = grid.saved_cursor {
                    grid.cursor.col = saved.col.min(grid.cols - 1);
                    grid.cursor.row = saved.row.min(grid.rows - 1);
                    grid.cursor.wrap_pending = false;
                    grid.active_fg = saved.fg;
                    grid.active_bg = saved.bg;
                    grid.active_flags = saved.flags;
                }
            }
            'c' => {
                if intermediates.is_empty() && p_raw(0, 0) == 0 {
                    // VT220 primary device attributes
                    self.pty_replies.extend_from_slice(b"\x1b[?62;22c");
                }
            }
            'n' => {
                match p_raw(0, 0) {
                    5 => self.pty_replies.extend_from_slice(b"\x1b[0n"),
                    6 => {
                        let grid = self.active_grid();
                        let reply = format!("\x1b[{};{}R", grid.cursor.row + 1, grid.cursor.col + 1);
                        self.pty_replies.extend_from_slice(reply.as_bytes());
                    }
                    _ => {}
                }
            }
            'm' => {
                self.apply_sgr(params);
            }
            _ => {}
        }

        self.dirty = true;
    }
}

impl Terminal {
    fn set_private_mode(&mut self, mode: u16, enable: bool) {
        match mode {
            1 => self.application_cursor_keys = enable,
            25 => self.active_grid_mut().cursor.visible = enable,
            47 | 1047 => self.set_alt_screen(enable, false),
            1049 => self.set_alt_screen(enable, true),
            2004 => self.bracketed_paste = enable,
            _ => {}
        }
    }

    fn apply_sgr(&mut self, params: &Params) {
        let grid = self.active_grid_mut();
        if params.is_empty() {
            grid.active_fg = Color::DefaultFg;
            grid.active_bg = Color::DefaultBg;
            grid.active_flags = CellFlags::default();
            return;
        }

        let mut iter = params.iter();
        while let Some(sub) = iter.next() {
            let code = sub.first().copied().unwrap_or(0);
            match code {
                0 => {
                    grid.active_fg = Color::DefaultFg;
                    grid.active_bg = Color::DefaultBg;
                    grid.active_flags = CellFlags::default();
                }
                1 => grid.active_flags.bold = true,
                2 => grid.active_flags.dim = true,
                3 => grid.active_flags.italic = true,
                4 => {
                    let sub_kind = sub.get(1).copied().unwrap_or(1);
                    grid.active_flags.underline = sub_kind != 0;
                }
                7 => grid.active_flags.inverse = true,
                8 => grid.active_flags.hidden = true,
                9 => grid.active_flags.strikethrough = true,
                21 => grid.active_flags.underline = true,
                22 => {
                    grid.active_flags.bold = false;
                    grid.active_flags.dim = false;
                }
                23 => grid.active_flags.italic = false,
                24 => grid.active_flags.underline = false,
                27 => grid.active_flags.inverse = false,
                28 => grid.active_flags.hidden = false,
                29 => grid.active_flags.strikethrough = false,
                30..=37 => grid.active_fg = Color::Indexed((code - 30) as u8),
                39 => grid.active_fg = Color::DefaultFg,
                40..=47 => grid.active_bg = Color::Indexed((code - 40) as u8),
                49 => grid.active_bg = Color::DefaultBg,
                90..=97 => grid.active_fg = Color::Indexed((code - 90 + 8) as u8),
                100..=107 => grid.active_bg = Color::Indexed((code - 100 + 8) as u8),
                38 | 48 | 58 => {
                    let parsed = if sub.len() >= 2 {
                        // Colon-delimited form: 38:5:N or 38:2:CS:R:G:B or 38:2:R:G:B
                        match sub[1] {
                            5 if sub.len() >= 3 => Some(Color::Indexed(sub[2] as u8)),
                            2 if sub.len() >= 6 => {
                                Some(Color::Rgb(sub[3] as u8, sub[4] as u8, sub[5] as u8))
                            }
                            2 if sub.len() >= 5 => {
                                Some(Color::Rgb(sub[2] as u8, sub[3] as u8, sub[4] as u8))
                            }
                            _ => None,
                        }
                    } else if let Some(kind_sub) = iter.next() {
                        // Semicolon-delimited form: 38;5;N or 38;2;R;G;B
                        match kind_sub.first().copied() {
                            Some(5) => iter
                                .next()
                                .and_then(|n| n.first().copied())
                                .map(|n| Color::Indexed(n as u8)),
                            Some(2) => {
                                let r = iter.next().and_then(|n| n.first().copied()).unwrap_or(0);
                                let g = iter.next().and_then(|n| n.first().copied()).unwrap_or(0);
                                let b = iter.next().and_then(|n| n.first().copied()).unwrap_or(0);
                                Some(Color::Rgb(r as u8, g as u8, b as u8))
                            }
                            _ => None,
                        }
                    } else {
                        None
                    };

                    if let Some(c) = parsed {
                        if code == 38 {
                            grid.active_fg = c;
                        } else if code == 48 {
                            grid.active_bg = c;
                        }
                        // code == 58 (underline color) is consumed cleanly without corrupting flags
                    }
                }
                59 => {}
                _ => {}
            }
        }
    }
}

#[allow(dead_code)]
fn _assert_cell_size() {
    let _ = std::mem::size_of::<Cell>();
}
