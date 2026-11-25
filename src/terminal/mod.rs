pub mod color;
pub mod grid;
mod parser;

use vte::Parser;

use self::grid::{CellFlags, Grid, Row};

pub struct Terminal {
    primary: Grid,
    alternate: Grid,
    using_alt_screen: bool,
    pub application_cursor_keys: bool,
    pub bracketed_paste: bool,
    pub pending_title: Option<String>,
    pub pty_replies: Vec<u8>,
    pub dirty: bool,
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
        self.dirty = true;
    }

    pub fn set_alt_screen(&mut self, enable: bool, clear_on_enter: bool) {
        if enable == self.using_alt_screen {
            return;
        }
        self.using_alt_screen = enable;
        if enable && clear_on_enter {
            let cols = self.alternate.cols;
            let rows = self.alternate.rows;
            self.alternate = Grid::new(cols, rows, 0);
            self.alternate.active_flags = CellFlags::default();
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
