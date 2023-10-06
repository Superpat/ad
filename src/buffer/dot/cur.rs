use crate::{
    buffer::{
        dot::util::{
            cond::Cond,
            consumer::{chain_consume, Consumer},
            iter::{IdxChars, IdxLines, RevIdxChars, RevIdxLines},
        },
        Buffer,
    },
    key::Arrow,
};
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
        Cur::from_char_idx(b.txt.len_chars(), b)
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
                    cur.x = b.txt.line(cur.y).len_chars().saturating_sub(1);
                }
            }
            Arrow::Right => {
                if let Some(line) = b.line(cur.y) {
                    match cur.x.cmp(&line.len_chars().saturating_sub(1)) {
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
            b.txt.line(self.y).len_chars().saturating_sub(1)
        };

        if self.x > len {
            self.x = len;
        }
    }

    fn set_x_from_buffer_rx(&mut self, b: &Buffer) {
        self.x = b.x_from_rx(self.y);
    }

    #[must_use]
    pub(super) fn move_to_line_start(mut self) -> Self {
        self.x = 0;
        self
    }

    #[must_use]
    pub(super) fn move_to_line_end(mut self, b: &Buffer) -> Self {
        self.x += b.txt.line(self.y).chars().skip(self.x).count();
        self.x = self.x.saturating_sub(1);
        self
    }

    /// Move this cursor forward by character through the buffer while the provided conditions are
    /// satisfied, returning a new Cur pointing to the index where they return Some(index). If the
    /// conditions never return Some then the resulting Cur will be EOF.
    #[must_use]
    pub(super) fn fwd_chars<'b, const C: usize>(
        self,
        b: &'b Buffer,
        conds: [(Consumer<IdxChars<'b>, char>, Cond<char>); C],
    ) -> Self {
        match chain_consume(IdxChars::new(self, b), conds) {
            Some(idx) => Cur::from_char_idx(idx, b),
            None => Cur::buffer_end(b),
        }
    }

    /// Move this cursor backward by character through the buffer while the provided conditions are
    /// satisfied, returning a new Cur pointing to the index where they return Some(index). If the
    /// conditions never return Some then the resulting Cur will be 0,0.
    #[must_use]
    pub(super) fn bwd_chars<'b, const C: usize>(
        self,
        b: &'b Buffer,
        conds: [(Consumer<RevIdxChars<'b>, char>, Cond<char>); C],
    ) -> Self {
        match chain_consume(RevIdxChars::new(self, b), conds) {
            Some(idx) => Cur::from_char_idx(idx, b),
            None => Cur::buffer_start(),
        }
    }

    /// Move this cursor forward by line through the buffer while the provided conditions are
    /// satisfied, returning a new Cur pointing to the index where they return Some(index). If the
    /// conditions never return Some then the resulting Cur will be EOF.
    #[must_use]
    pub(super) fn fwd_lines<'b, const C: usize>(
        self,
        b: &'b Buffer,
        conds: [(Consumer<IdxLines<'b>, RopeSlice<'b>>, Cond<RopeSlice<'b>>); C],
    ) -> Self {
        match chain_consume(IdxLines::new(self, b), conds) {
            Some(y) => Cur { y, x: 0 },
            None => Cur::buffer_end(b),
        }
    }

    /// Move this cursor backward by line through the buffer while the provided conditions are
    /// satisfied, returning a new Cur pointing to the index where they return Some(index). If the
    /// conditions never return Some then the resulting Cur will be 0,0.
    #[must_use]
    pub(super) fn bwd_lines<'b, const C: usize>(
        self,
        b: &'b Buffer,
        conds: [(
            Consumer<RevIdxLines<'b>, RopeSlice<'b>>,
            Cond<RopeSlice<'b>>,
        ); C],
    ) -> Self {
        match chain_consume(RevIdxLines::new(self, b), conds) {
            Some(y) => Cur { y, x: 0 },
            None => Cur::buffer_start(),
        }
    }
}
