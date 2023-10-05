use crate::{buffer::Buffer, key::Arrow};
use ropey::RopeSlice;
use std::{cmp::Ordering, fmt};

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cur {
    pub y: usize,
    pub x: usize,
}

impl fmt::Display for Cur {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.y + 1, self.x + 1)
    }
}

impl Cur {
    pub fn buffer_start() -> Self {
        Cur { y: 0, x: 0 }
    }

    pub fn buffer_end(b: &Buffer) -> Self {
        Cur {
            y: b.txt.len_lines(),
            x: 0,
        }
    }

    pub(crate) fn as_char_idx(&self, b: &Buffer) -> usize {
        b.txt.line_to_char(self.y) + self.x
    }

    pub(crate) fn from_char_idx(idx: usize, b: &Buffer) -> Self {
        let y = b.txt.char_to_line(idx);
        let x = idx - b.txt.line_to_char(y);

        Self { y, x }
    }

    pub(super) fn arr_w_count(&self, arr: Arrow, count: usize, b: &Buffer) -> Self {
        let mut cur = *self;

        for _ in 0..count {
            cur = cur.arr(arr, b);
        }

        cur.clamp_x(b);
        cur
    }

    #[must_use]
    pub(super) fn move_to_line_start(mut self) -> Self {
        self.x = 0;
        self
    }

    #[must_use]
    pub(super) fn move_to_line_end(mut self, b: &Buffer) -> Self {
        self.x += b.txt.line(self.y).chars().skip(self.x).count();
        self
    }

    /// Move forward until cond returns an x position in the given line or we bottom out at the end of the buffer
    #[must_use]
    pub(super) fn move_to(mut self, b: &Buffer, cond: fn(RopeSlice) -> Option<usize>) -> Self {
        for line in b.txt.lines().skip(self.y + 1) {
            self.y += 1;
            if let Some(x) = (cond)(line) {
                self.x = x;
                return self;
            }
        }
        self.move_to_line_end(b)
    }

    /// Move back until cond returns an x position in the given line or we bottom out at the start of the buffer
    #[must_use]
    pub(super) fn move_back_to(mut self, b: &Buffer, cond: fn(RopeSlice) -> Option<usize>) -> Self {
        // Ropey::Lines isn't double ended so we need to collect which is unfortunate
        let lines: Vec<RopeSlice> = b.txt.lines().take(self.y).collect();
        for line in lines.into_iter().rev() {
            self.y -= 1;
            if let Some(x) = (cond)(line) {
                self.x = x;
                return self;
            }
        }
        self.move_to_line_start()
    }

    fn arr(&self, arr: Arrow, b: &Buffer) -> Self {
        let mut cur = *self;

        match arr {
            Arrow::Up => {
                if cur.y != 0 {
                    cur.y -= 1;
                    cur.set_x_from_buffer_rx(b);
                }
            }
            Arrow::Down => {
                if !b.is_empty() && cur.y < b.len_lines() - 1 {
                    cur.y += 1;
                    cur.set_x_from_buffer_rx(b);
                }
            }
            Arrow::Left => {
                if cur.x != 0 {
                    cur.x -= 1;
                } else if cur.y > 0 {
                    // Allow <- to move to the end of the previous line
                    cur.y -= 1;
                    cur.x = b.txt.line(cur.y).len_chars();
                }
            }
            Arrow::Right => {
                if let Some(line) = b.line(cur.y) {
                    match cur.x.cmp(&line.len_chars()) {
                        Ordering::Less => cur.x += 1,
                        Ordering::Equal => {
                            // Allow -> to move to the start of the next line
                            cur.y += 1;
                            cur.x = 0;
                        }
                        _ => (),
                    }
                }
            }
        }

        cur
    }

    fn clamp_x(&mut self, b: &Buffer) {
        let len = if self.y >= b.len_lines() {
            0
        } else {
            b.txt.line(self.y).len_chars()
        };

        if self.x > len {
            self.x = len;
        }
    }

    fn set_x_from_buffer_rx(&mut self, b: &Buffer) {
        self.x = b.x_from_rx(self.y);
    }
}
