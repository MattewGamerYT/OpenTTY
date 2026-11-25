use std::collections::VecDeque;

use super::color::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorStyle {
    Block,
    Beam,
    Underline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellFlags {
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
    pub hidden: bool,
    pub strikethrough: bool,
}

impl Default for CellFlags {
    fn default() -> Self {
        Self {
            bold: false,
            dim: false,
            italic: false,
            underline: false,
            inverse: false,
            hidden: false,
            strikethrough: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub flags: CellFlags,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: Color::DefaultFg,
            bg: Color::DefaultBg,
            flags: CellFlags::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Row {
    pub cells: Vec<Cell>,
}

impl Row {
    pub fn new(cols: usize) -> Self {
        Self {
            cells: vec![Cell::default(); cols],
        }
    }

    pub fn clear(&mut self, template: Cell) {
        self.cells.fill(template);
    }

    pub fn resize(&mut self, cols: usize) {
        self.cells.resize(cols, Cell::default());
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Cursor {
    pub col: usize,
    pub row: usize,
    pub visible: bool,
    pub style: CursorStyle,
    pub wrap_pending: bool,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            col: 0,
            row: 0,
            visible: true,
            style: CursorStyle::Block,
            wrap_pending: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SavedCursor {
    pub col: usize,
    pub row: usize,
    pub fg: Color,
    pub bg: Color,
    pub flags: CellFlags,
}

pub struct Grid {
    pub cols: usize,
    pub rows: usize,
    pub lines: Vec<Row>,
    pub scrollback: VecDeque<Row>,
    pub max_scrollback: usize,
    pub scroll_offset: usize,
    pub scroll_top: usize,
    pub scroll_bottom: usize,
    pub cursor: Cursor,
    pub active_fg: Color,
    pub active_bg: Color,
    pub active_flags: CellFlags,
    pub saved_cursor: Option<SavedCursor>,
}

impl Grid {
    pub fn new(cols: usize, rows: usize, max_scrollback: usize) -> Self {
        let cols = cols.max(1);
        let rows = rows.max(1);
        let lines = (0..rows).map(|_| Row::new(cols)).collect();
        Self {
            cols,
            rows,
            lines,
            scrollback: VecDeque::new(),
            max_scrollback,
            scroll_offset: 0,
            scroll_top: 0,
            scroll_bottom: rows - 1,
            cursor: Cursor::default(),
            active_fg: Color::DefaultFg,
            active_bg: Color::DefaultBg,
            active_flags: CellFlags::default(),
            saved_cursor: None,
        }
    }

    pub fn blank_cell(&self) -> Cell {
        Cell {
            ch: ' ',
            fg: self.active_fg,
            bg: self.active_bg,
            flags: CellFlags::default(),
        }
    }

    pub fn resize(&mut self, new_cols: usize, new_rows: usize) {
        let new_cols = new_cols.max(1);
        let new_rows = new_rows.max(1);

        if new_cols != self.cols {
            for row in &mut self.lines {
                row.resize(new_cols);
            }
            for row in &mut self.scrollback {
                row.resize(new_cols);
            }
            self.cols = new_cols;
        }

        if new_rows < self.rows {
            let excess = self.rows - new_rows;
            // Push top lines into scrollback when shrinking vertically so prompt stays visible.
            for _ in 0..excess {
                if self.cursor.row >= new_rows {
                    let evicted = self.lines.remove(0);
                    self.push_scrollback(evicted);
                    self.cursor.row = self.cursor.row.saturating_sub(1);
                } else {
                    self.lines.pop();
                }
            }
        } else if new_rows > self.rows {
            let extra = new_rows - self.rows;
            for _ in 0..extra {
                self.lines.push(Row::new(self.cols));
            }
        }

        self.rows = new_rows;
        self.scroll_top = 0;
        self.scroll_bottom = new_rows - 1;
        self.cursor.col = self.cursor.col.min(self.cols - 1);
        self.cursor.row = self.cursor.row.min(self.rows - 1);
        self.cursor.wrap_pending = false;
        self.scroll_offset = self.scroll_offset.min(self.scrollback.len());
    }

    pub fn push_scrollback(&mut self, row: Row) {
        if self.max_scrollback == 0 {
            return;
        }
        if self.scrollback.len() >= self.max_scrollback {
            self.scrollback.pop_front();
        }
        self.scrollback.push_back(row);
    }

    pub fn scroll_up_in_region(&mut self, count: usize, save_to_scrollback: bool) {
        let top = self.scroll_top.min(self.rows - 1);
        let bottom = self.scroll_bottom.min(self.rows - 1);
        if top > bottom {
            return;
        }
        let region_len = bottom - top + 1;
        let steps = count.min(region_len);
        let template = self.blank_cell();

        for _ in 0..steps {
            let removed = self.lines.remove(top);
            if save_to_scrollback && top == 0 && bottom == self.rows - 1 {
                self.push_scrollback(removed);
            }
            let mut blank = Row::new(self.cols);
            blank.clear(template);
            self.lines.insert(bottom, blank);
        }
    }

    pub fn scroll_down_in_region(&mut self, count: usize) {
        let top = self.scroll_top.min(self.rows - 1);
        let bottom = self.scroll_bottom.min(self.rows - 1);
        if top > bottom {
            return;
        }
        let region_len = bottom - top + 1;
        let steps = count.min(region_len);
        let template = self.blank_cell();

        for _ in 0..steps {
            self.lines.remove(bottom);
            let mut blank = Row::new(self.cols);
            blank.clear(template);
            self.lines.insert(top, blank);
        }
    }

    pub fn put_char(&mut self, ch: char) {
        self.scroll_offset = 0;
        if self.cursor.wrap_pending {
            self.cursor.wrap_pending = false;
            self.cursor.col = 0;
            self.linefeed();
        }

        let row = self.cursor.row.min(self.rows - 1);
        let col = self.cursor.col.min(self.cols - 1);
        self.lines[row].cells[col] = Cell {
            ch,
            fg: self.active_fg,
            bg: self.active_bg,
            flags: self.active_flags,
        };

        if self.cursor.col + 1 >= self.cols {
            self.cursor.wrap_pending = true;
        } else {
            self.cursor.col += 1;
        }
    }

    pub fn linefeed(&mut self) {
        self.cursor.wrap_pending = false;
        if self.cursor.row == self.scroll_bottom {
            self.scroll_up_in_region(1, true);
        } else if self.cursor.row + 1 < self.rows {
            self.cursor.row += 1;
        }
    }

    pub fn reverse_index(&mut self) {
        self.cursor.wrap_pending = false;
        if self.cursor.row == self.scroll_top {
            self.scroll_down_in_region(1);
        } else if self.cursor.row > 0 {
            self.cursor.row -= 1;
        }
    }

    pub fn visible_row(&self, screen_row: usize) -> Option<&Row> {
        if self.scroll_offset == 0 {
            return self.lines.get(screen_row);
        }
        let sb_len = self.scrollback.len();
        let total_index = sb_len + screen_row;
        let shifted = total_index.checked_sub(self.scroll_offset)?;
        if shifted < sb_len {
            self.scrollback.get(shifted)
        } else {
            self.lines.get(shifted - sb_len)
        }
    }

    pub fn scroll_view(&mut self, delta_lines: isize) {
        let max_offset = self.scrollback.len();
        if delta_lines > 0 {
            self.scroll_offset = (self.scroll_offset + delta_lines as usize).min(max_offset);
        } else {
            self.scroll_offset = self.scroll_offset.saturating_sub((-delta_lines) as usize);
        }
    }
}
