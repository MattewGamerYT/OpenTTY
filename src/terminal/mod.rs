pub mod color;
pub mod grid;
mod parser;

use vte::Parser;

use self::grid::{CellFlags, Grid, Row, SavedCursor};

pub struct Terminal {
    primary: Grid,
    alternate: Grid,
    using_alt_screen: bool,
    pub application_cursor_keys: bool,
    pub bracketed_paste: bool,
    pub pending_title: Option<String>,
    pub pty_replies: Vec<u8>,
    pub dirty: bool,
    pub last_printed_char: Option<char>,
    pub pixel_width: u16,
    pub pixel_height: u16,
    max_scrollback: usize,
}

pub struct TerminalState {
    parser: Parser,
    pub term: Terminal,
}

impl TerminalState {
    pub fn new(cols: usize, rows: usize, max_scrollback: usize) -> Self {
        Self {
            parser: Parser::new(),
            term: Terminal::new(cols, rows, max_scrollback),
        }
    }

    pub fn feed_bytes(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }
}

impl Terminal {
    pub fn new(cols: usize, rows: usize, max_scrollback: usize) -> Self {
        Self {
            primary: Grid::new(cols, rows, max_scrollback),
            alternate: Grid::new(cols, rows, 0),
            using_alt_screen: false,
            application_cursor_keys: false,
            bracketed_paste: false,
            pending_title: None,
            pty_replies: Vec::new(),
            dirty: true,
            last_printed_char: None,
            pixel_width: 0,
            pixel_height: 0,
            max_scrollback,
        }
    }

    pub fn cols(&self) -> usize {
        self.active_grid().cols
    }

    pub fn rows(&self) -> usize {
        self.active_grid().rows
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
        self.dirty = true;
    }

    pub fn reset(&mut self) {
        let cols = self.cols();
        let rows = self.rows();
        self.primary = Grid::new(cols, rows, self.max_scrollback);
        self.alternate = Grid::new(cols, rows, 0);
        self.using_alt_screen = false;
        self.application_cursor_keys = false;
        self.bracketed_paste = false;
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
                self.alternate.active_flags = CellFlags::default();
            }
        } else {
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
}
