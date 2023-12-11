use crate::{
    buffer::GapBuffer,
    dot::Dot,
    exec::{addr::Address, Edit},
};
use std::{
    cell::RefCell,
    io::{stdin, Stdin},
};

/// Initial length of the line buffer for when we read from stdin
const LINE_BUF_LEN: usize = 100;

/// A wrapper around stdin that buffers and caches input so that it can be manipulated
/// like a Buffer
pub struct CachedStdin {
    inner: RefCell<CachedStdinInner>,
    buf: RefCell<String>,
    gb: RefCell<GapBuffer>,
}

struct CachedStdinInner {
    stdin: Stdin,
    closed: bool,
}

impl Default for CachedStdin {
    fn default() -> Self {
        Self::new()
    }
}

impl CachedStdin {
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(CachedStdinInner {
                stdin: stdin(),
                closed: false,
            }),
            buf: RefCell::new(String::with_capacity(LINE_BUF_LEN)),
            gb: RefCell::new(GapBuffer::from("")),
        }
    }

    fn is_closed(&self) -> bool {
        self.inner.borrow().closed
    }

    fn get_char(&self, ix: usize) -> Option<char> {
        self.gb.borrow().get_char(ix)
    }

    fn try_read_next_line(&self) {
        let mut inner = self.inner.borrow_mut();
        let mut buf = self.buf.borrow_mut();
        buf.clear();

        match inner.stdin.read_line(&mut buf) {
            Ok(n) => {
                let len = self.gb.borrow().len_chars();
                self.gb.borrow_mut().insert_str(len, &buf);
                inner.closed = n == 0;
            }
            Err(_) => inner.closed = true,
        };
    }
}

impl Address for CachedStdin {
    fn current_dot(&self) -> Dot {
        Dot::default()
    }

    fn len_chars(&self) -> usize {
        self.gb.borrow().len_chars()
    }

    fn line_to_char(&self, line_idx: usize) -> Option<usize> {
        let r = self.gb.borrow();
        let cur_len = r.len_lines();
        if line_idx > cur_len {
            for _ in cur_len..=line_idx {
                self.try_read_next_line();
                if self.is_closed() {
                    break;
                }
            }
        }

        r.try_line_to_char(line_idx)
    }

    fn char_to_line(&self, char_idx: usize) -> Option<usize> {
        self.gb.borrow().try_char_to_line(char_idx)
    }

    fn char_to_line_end(&self, char_idx: usize) -> Option<usize> {
        let gb = self.gb.borrow();
        let line_idx = gb.try_char_to_line(char_idx)?;
        Some(gb.line_to_char(line_idx) + gb.line(line_idx).chars().count())
    }

    fn char_to_line_start(&self, char_idx: usize) -> Option<usize> {
        let gb = self.gb.borrow();
        let line_idx = gb.try_char_to_line(char_idx)?;
        Some(gb.line_to_char(line_idx))
    }
}

impl Edit for CachedStdin {
    fn contents(&self) -> String {
        self.gb.borrow().to_string()
    }

    fn insert(&mut self, ix: usize, s: &str) {
        self.gb.borrow_mut().insert_str(ix, s)
    }

    fn remove(&mut self, from: usize, to: usize) {
        self.gb.borrow_mut().remove_range(from, to)
    }
}

pub struct CachedStdinIter<'a> {
    pub(super) inner: &'a CachedStdin,
    pub(super) from: usize,
    pub(super) to: usize,
}

impl<'a> Iterator for CachedStdinIter<'a> {
    type Item = (usize, char);

    fn next(&mut self) -> Option<Self::Item> {
        // self.from == self.to - 1 is the last character so
        // we catch end of iteration on the subsequent call
        if self.from >= self.to {
            return None;
        }

        loop {
            if self.inner.is_closed() {
                return None;
            }

            match self.inner.get_char(self.from) {
                Some(ch) => {
                    let res = (self.from, ch);
                    self.from += 1;

                    return Some(res);
                }
                None => self.inner.try_read_next_line(),
            }
        }
    }
}
