pub mod color;
pub mod grid;
mod parser;

use std::time::{Duration, Instant};

use vte::Parser;

use self::grid::{CellFlags, CursorStyle, Grid, Row, SavedCursor, Selection, SelectionMode, SelectionPoint};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseMode {
    Off,
    Normal,
    ButtonEvent,
    AnyEvent,
}

pub struct Terminal {
    primary: Grid,
    alternate: Grid,
    using_alt_screen: bool,
    pub application_cursor_keys: bool,
    pub bracketed_paste: bool,
    pub mouse_mode: MouseMode,
    pub sgr_mouse: bool,
    pub focus_reporting: bool,
    pub sync_update: bool,
    pub sync_started_at: Option<Instant>,
    pub pending_title: Option<String>,
    pub pending_clipboard: Option<String>,
    pub pty_replies: Vec<u8>,
    pub dirty: bool,
    pub last_printed_char: Option<char>,
    pub pixel_width: u16,
    pub pixel_height: u16,
    pub default_cursor_style: CursorStyle,
    max_scrollback: usize,
    last_click_at: Option<Instant>,
    last_click_pos: Option<SelectionPoint>,
    click_streak: u8,
}

pub struct TerminalState {
    parser: Parser,
    pub term: Terminal,
}

impl TerminalState {
    pub fn new(
        cols: usize,
        rows: usize,
        max_scrollback: usize,
        default_cursor_style: CursorStyle,
    ) -> Self {
        Self {
            parser: Parser::new(),
            term: Terminal::new(cols, rows, max_scrollback, default_cursor_style),
        }
    }

    pub fn feed_bytes(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }
}

impl Terminal {
    pub fn new(
        cols: usize,
        rows: usize,
        max_scrollback: usize,
        default_cursor_style: CursorStyle,
    ) -> Self {
        let mut primary = Grid::new(cols, rows, max_scrollback);
        primary.cursor.style = default_cursor_style;
        let mut alternate = Grid::new(cols, rows, 0);
        alternate.cursor.style = default_cursor_style;
        Self {
            primary,
            alternate,
            using_alt_screen: false,
            application_cursor_keys: false,
            bracketed_paste: false,
            mouse_mode: MouseMode::Off,
            sgr_mouse: false,
            focus_reporting: false,
            sync_update: false,
            sync_started_at: None,
            pending_title: None,
            pending_clipboard: None,
            pty_replies: Vec::new(),
            dirty: true,
            last_printed_char: None,
            pixel_width: 0,
            pixel_height: 0,
            default_cursor_style,
            max_scrollback,
            last_click_at: None,
            last_click_pos: None,
            click_streak: 0,
        }
    }

    pub fn cols(&self) -> usize {
        self.active_grid().cols
    }

    pub fn rows(&self) -> usize {
        self.active_grid().rows
    }

    pub fn is_alt_screen(&self) -> bool {
        self.using_alt_screen
    }

    pub fn mouse_reporting_active(&self) -> bool {
        self.mouse_mode != MouseMode::Off
    }

    pub fn active_grid(&self) -> &Grid {
        if self.using_alt_screen {
            &self.alternate
        } else {
            &self.primary
        }
    }

    pub fn active_grid_mut(&mut self) -> &mut Grid {
        if self.using_alt_screen {
            &mut self.alternate
        } else {
            &mut self.primary
        }
    }

    pub fn set_pixel_size(&mut self, width: u16, height: u16) {
        self.pixel_width = width;
        self.pixel_height = height;
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.primary.resize_reflow(cols, rows);
        self.alternate.resize(cols, rows);
        self.primary.selection = None;
        self.alternate.selection = None;
        self.dirty = true;
    }

    pub fn reset(&mut self) {
        let cols = self.cols();
        let rows = self.rows();
        self.primary = Grid::new(cols, rows, self.max_scrollback);
        self.primary.cursor.style = self.default_cursor_style;
        self.alternate = Grid::new(cols, rows, 0);
        self.alternate.cursor.style = self.default_cursor_style;
        self.using_alt_screen = false;
        self.application_cursor_keys = false;
        self.bracketed_paste = false;
        self.mouse_mode = MouseMode::Off;
        self.sgr_mouse = false;
        self.focus_reporting = false;
        self.sync_update = false;
        self.sync_started_at = None;
        self.last_printed_char = None;
        self.dirty = true;
    }

    pub fn save_cursor(&mut self) {
        let grid = self.active_grid_mut();
        grid.saved_cursor = Some(SavedCursor {
            col: grid.cursor.col,
            row: grid.cursor.row,
            fg: grid.active_fg,
            bg: grid.active_bg,
            flags: grid.active_flags,
        });
    }

    pub fn restore_cursor(&mut self) {
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

    pub fn set_alt_screen(&mut self, enable: bool, clear_and_save_cursor: bool) {
        if enable == self.using_alt_screen {
            return;
        }
        if enable {
            if clear_and_save_cursor {
                self.primary.saved_cursor = Some(SavedCursor {
                    col: self.primary.cursor.col,
                    row: self.primary.cursor.row,
                    fg: self.primary.active_fg,
                    bg: self.primary.active_bg,
                    flags: self.primary.active_flags,
                });
            }
            self.using_alt_screen = true;
            if clear_and_save_cursor {
                let cols = self.alternate.cols;
                let rows = self.alternate.rows;
                self.alternate = Grid::new(cols, rows, 0);
                self.alternate.cursor.style = self.default_cursor_style;
                self.alternate.active_flags = CellFlags::default();
            }
        } else {
            self.alternate.selection = None;
            self.using_alt_screen = false;
            if clear_and_save_cursor && let Some(saved) = self.primary.saved_cursor {
                self.primary.cursor.col = saved.col.min(self.primary.cols.saturating_sub(1));
                self.primary.cursor.row = saved.row.min(self.primary.rows.saturating_sub(1));
                self.primary.cursor.wrap_pending = false;
                self.primary.active_fg = saved.fg;
                self.primary.active_bg = saved.bg;
                self.primary.active_flags = saved.flags;
            }
        }
        self.dirty = true;
    }

    pub fn visible_row(&self, row: usize) -> Option<&Row> {
        self.active_grid().visible_row(row)
    }

    pub fn scroll_view(&mut self, delta_lines: isize) {
        if !self.using_alt_screen {
            self.primary.scroll_view(delta_lines);
            self.dirty = true;
        }
    }

    pub fn start_selection(&mut self, col: usize, row: usize) {
        let now = Instant::now();
        let (abs_row, col, pt) = {
            let grid = self.active_grid();
            let abs_row = grid.abs_row_for_visible(row.min(grid.rows.saturating_sub(1)));
            let col = col.min(grid.cols.saturating_sub(1));
            (abs_row, col, SelectionPoint { abs_row, col })
        };

        let multi_click = self
            .last_click_at
            .is_some_and(|t| now.duration_since(t) <= Duration::from_millis(400))
            && self.last_click_pos == Some(pt);

        if multi_click {
            self.click_streak = (self.click_streak % 3) + 1;
        } else {
            self.click_streak = 1;
        }
        self.last_click_at = Some(now);
        self.last_click_pos = Some(pt);

        let mode = match self.click_streak {
            2 => SelectionMode::Word,
            3 => SelectionMode::Line,
            _ => SelectionMode::Char,
        };

        let grid = self.active_grid_mut();
        let sel = match mode {
            SelectionMode::Char => Selection {
                anchor: pt,
                focus: pt,
                mode,
                active: false,
            },
            SelectionMode::Word => {
                let (s, e) = grid.word_bounds_at(abs_row, col);
                Selection {
                    anchor: SelectionPoint { abs_row, col: s },
                    focus: SelectionPoint { abs_row, col: e },
                    mode,
                    active: true,
                }
            }
            SelectionMode::Line => {
                let (r0, r1) = grid.logical_line_bounds(abs_row);
                Selection {
                    anchor: SelectionPoint { abs_row: r0, col: 0 },
                    focus: SelectionPoint {
                        abs_row: r1,
                        col: grid.cols.saturating_sub(1),
                    },
                    mode,
                    active: true,
                }
            }
        };

        grid.selection = Some(sel);
        self.dirty = true;
    }

    pub fn update_selection(&mut self, col: usize, row: usize) {
        let grid = self.active_grid_mut();
        let Some(mut sel) = grid.selection else {
            return;
        };
        let abs_row = grid.abs_row_for_visible(row.min(grid.rows.saturating_sub(1)));
        let col = col.min(grid.cols.saturating_sub(1));
        let pt = SelectionPoint { abs_row, col };

        match sel.mode {
            SelectionMode::Char => {
                if pt != sel.anchor {
                    sel.active = true;
                }
                sel.focus = pt;
            }
            SelectionMode::Word => {
                let (orig_s, orig_e) = grid.word_bounds_at(sel.anchor.abs_row, sel.anchor.col);
                let (s, e) = grid.word_bounds_at(abs_row, col);
                if abs_row < sel.anchor.abs_row
                    || (abs_row == sel.anchor.abs_row && e < orig_s)
                {
                    sel.anchor = SelectionPoint {
                        abs_row: sel.anchor.abs_row,
                        col: orig_e,
                    };
                    sel.focus = SelectionPoint { abs_row, col: s };
                } else if abs_row > sel.anchor.abs_row
                    || (abs_row == sel.anchor.abs_row && s > orig_e)
                {
                    sel.anchor = SelectionPoint {
                        abs_row: sel.anchor.abs_row,
                        col: orig_s,
                    };
                    sel.focus = SelectionPoint { abs_row, col: e };
                } else {
                    sel.anchor = SelectionPoint {
                        abs_row: sel.anchor.abs_row,
                        col: orig_s,
                    };
                    sel.focus = SelectionPoint {
                        abs_row: sel.anchor.abs_row,
                        col: orig_e,
                    };
                }
                sel.active = true;
            }
            SelectionMode::Line => {
                let (orig_r0, orig_r1) = grid.logical_line_bounds(sel.anchor.abs_row);
                let (r0, r1) = grid.logical_line_bounds(abs_row);
                let max_col = grid.cols.saturating_sub(1);
                if r1 < orig_r0 {
                    sel.anchor = SelectionPoint {
                        abs_row: orig_r1,
                        col: max_col,
                    };
                    sel.focus = SelectionPoint {
                        abs_row: r0,
                        col: 0,
                    };
                } else if r0 > orig_r1 {
                    sel.anchor = SelectionPoint {
                        abs_row: orig_r0,
                        col: 0,
                    };
                    sel.focus = SelectionPoint {
                        abs_row: r1,
                        col: max_col,
                    };
                } else {
                    sel.anchor = SelectionPoint {
                        abs_row: orig_r0,
                        col: 0,
                    };
                    sel.focus = SelectionPoint {
                        abs_row: orig_r1,
                        col: max_col,
                    };
                }
                sel.active = true;
            }
        }

        grid.selection = Some(sel);
        self.dirty = true;
    }

    pub fn finish_selection(&mut self) {
        let grid = self.active_grid_mut();
        if let Some(sel) = grid.selection
            && !sel.active
        {
            grid.selection = None;
            self.dirty = true;
        }
    }

    pub fn clear_selection(&mut self) {
        let grid = self.active_grid_mut();
        if grid.selection.take().is_some() {
            self.dirty = true;
        }
    }

    pub fn is_cell_selected(&self, abs_row: usize, col: usize) -> bool {
        self.active_grid()
            .selection
            .is_some_and(|sel| sel.contains(abs_row, col))
    }

    pub fn selected_text(&self, strip_trailing_spaces: bool) -> Option<String> {
        self.active_grid().extract_selection_text(strip_trailing_spaces)
    }

    pub fn sync_timed_out(&mut self) -> bool {
        if !self.sync_update {
            return false;
        }
        if let Some(started) = self.sync_started_at
            && started.elapsed() >= Duration::from_millis(50)
        {
            self.sync_update = false;
            self.sync_started_at = None;
            return true;
        }
        false
    }
}

