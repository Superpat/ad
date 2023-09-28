use crate::{
    key::{Arrow, Key},
    MAX_NAME_LEN, TAB_STOP, UNNAMED_BUFFER,
};
use std::{
    cmp::{min, Ordering},
    fs, io,
    path::{Path, PathBuf},
};

mod buffers;
mod line;

pub use buffers::Buffers;
pub use line::Line;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BufferKind {
    File(PathBuf),
    Virtual(String),
    Unnamed,
}

impl Default for BufferKind {
    fn default() -> Self {
        Self::Unnamed
    }
}

impl BufferKind {
    fn display_name(&self) -> Option<&str> {
        match self {
            BufferKind::File(p) => p.file_name()?.to_str(),
            BufferKind::Virtual(s) => Some(s.as_str()),
            BufferKind::Unnamed => Some(UNNAMED_BUFFER),
        }
    }
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct Buffer {
    pub(crate) kind: BufferKind,
    pub(crate) lines: Vec<Line>,
    pub(crate) cx: usize,
    pub(crate) cy: usize,
    pub(crate) rx: usize,
    pub(crate) row_off: usize,
    pub(crate) col_off: usize,
    pub(crate) dirty: bool,
}

impl Buffer {
    pub fn new_from_file(path: &str) -> io::Result<Self> {
        let raw = fs::read_to_string(path)?;
        let lines: Vec<Line> = raw.lines().map(|s| Line::new(s.to_string())).collect();

        Ok(Self {
            kind: BufferKind::File(PathBuf::from(path)),
            lines,
            cx: 0,
            cy: 0,
            rx: 0,
            row_off: 0,
            col_off: 0,
            dirty: false,
        })
    }

    pub fn new_virtual(name: String) -> Self {
        Self {
            kind: BufferKind::Virtual(name),
            lines: Vec::new(),
            cx: 0,
            cy: 0,
            rx: 0,
            row_off: 0,
            col_off: 0,
            dirty: false,
        }
    }

    pub fn display_name(&self) -> Option<&str> {
        let s = self.kind.display_name()?;

        Some(&s[0..min(MAX_NAME_LEN, s.len())])
    }

    pub fn file_path(&self) -> Option<&Path> {
        match &self.kind {
            BufferKind::File(p) => Some(p),
            _ => None,
        }
    }

    pub fn is_unnamed(&self) -> bool {
        self.kind == BufferKind::Unnamed
    }

    pub fn contents(&self) -> String {
        let mut s = String::new();
        for line in self.lines.iter() {
            s.push_str(&line.raw);
            s.push('\n');
        }

        s
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn clamp_scroll(&mut self, screen_rows: usize, screen_cols: usize) {
        self.rx = 0;
        if self.cy < self.lines.len() {
            self.update_rx();
        }

        if self.cy < self.row_off {
            self.row_off = self.cy;
        }

        if self.cy >= self.row_off + screen_rows {
            self.row_off = self.cy - screen_rows + 1;
        }

        if self.rx < self.col_off {
            self.col_off = self.rx;
        }

        if self.rx >= self.col_off + screen_cols {
            self.col_off = self.rx - screen_cols + 1;
        }
    }

    fn update_rx(&mut self) {
        let mut rx = 0;

        for c in self.lines[self.cy].raw.chars().take(self.cx) {
            if c == '\t' {
                rx += (TAB_STOP - 1) - (rx % TAB_STOP);
            }
            rx += 1;
        }

        self.rx = rx;
    }

    pub fn clamp_cx(&mut self) {
        let len = if self.cy >= self.len() {
            0
        } else {
            self.lines[self.cy].len()
        };

        if self.cx > len {
            self.cx = len;
        }
    }

    pub fn current_line(&self) -> Option<&Line> {
        if self.cy >= self.len() {
            None
        } else {
            Some(&self.lines[self.cy])
        }
    }

    pub fn handle_keypress(&mut self, k: Key, screen_rows: usize) -> io::Result<()> {
        match k {
            Key::Arrow(arr) => self.move_cursor(arr),
            Key::Home => self.cx = 0,
            Key::End => {
                if self.cy < self.lines.len() {
                    self.cx = self.lines[self.cy].len();
                }
            }
            Key::PageUp | Key::PageDown => {
                let arr = if k == Key::PageUp {
                    Arrow::Up
                } else {
                    Arrow::Down
                };

                for _ in 0..screen_rows {
                    self.move_cursor(arr);
                }
            }

            Key::Return => self.insert_newline(),

            Key::Backspace | Key::Del | Key::Ctrl('h') => {
                if k == Key::Del {
                    self.move_cursor(Arrow::Right);
                }
                self.delete_char();
            }

            Key::Tab => self.insert_char('\t'),
            Key::Char(c) => self.insert_char(c),

            _ => (),
        }

        Ok(())
    }

    fn move_cursor(&mut self, arr: Arrow) {
        match arr {
            Arrow::Up => {
                if self.cy != 0 {
                    self.cy -= 1;
                }
            }
            Arrow::Down => {
                if self.cy < self.lines.len() {
                    self.cy += 1;
                }
            }
            Arrow::Left => {
                if self.cx != 0 {
                    self.cx -= 1;
                } else if self.cy > 0 {
                    // Allow <- to move to the end of the previous line
                    self.cy -= 1;
                    self.cx = self.lines[self.cy].len();
                }
            }
            Arrow::Right => {
                if let Some(line) = self.current_line() {
                    match self.cx.cmp(&line.len()) {
                        Ordering::Less => self.cx += 1,
                        Ordering::Equal => {
                            // Allow -> to move to the start of the next line
                            self.cy += 1;
                            self.cx = 0;
                        }
                        _ => (),
                    }
                }
            }
        }

        self.clamp_cx();
    }

    fn insert_char(&mut self, c: char) {
        if self.cy == self.lines.len() {
            self.insert_line(self.lines.len(), String::new());
        }

        self.lines[self.cy].modify(|s| s.insert(self.cx, c));
        self.cx += 1;
        self.dirty = true;
    }

    fn insert_line(&mut self, at: usize, line: String) {
        if at <= self.len() {
            self.lines.insert(at, Line::new(line));
            self.dirty = true;
        }
    }

    fn insert_newline(&mut self) {
        if self.cx == 0 {
            self.insert_line(self.cy, String::new());
        } else {
            let (cur, nxt) = self.lines[self.cy].raw.split_at(self.cx);
            let (cur, nxt) = (cur.to_string(), nxt.to_string());
            self.lines[self.cy].modify(|s| *s = cur.clone());
            self.insert_line(self.cy + 1, nxt);
        }

        self.cy += 1;
        self.cx = 0;
        self.dirty = true;
    }

    fn delete_char(&mut self) {
        if self.cy == self.len() || (self.cx == 0 && self.cy == 0) {
            return;
        }

        if self.cx > 0 {
            self.lines[self.cy].modify(|s| {
                s.remove(self.cx - 1);
            });
            self.cx -= 1;
        } else {
            self.cx = self.lines[self.cy - 1].len();
            let line = self.lines.remove(self.cy);
            self.lines[self.cy - 1].modify(|s| s.push_str(&line.raw));
            self.cy -= 1;
        }

        self.dirty = true;
    }
}
