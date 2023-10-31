use super::vm::Regex;
use crate::util::IdxRopeChars;
use ropey::{Rope, RopeSlice};
use std::{
    iter::{Enumerate, Skip},
    str::Chars,
};

/// The match location of a Regex against a given input.
///
/// The sub-match indices are relative to the input used to run the original match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub(super) sub_matches: [usize; 20],
}

impl Match {
    pub(crate) fn synthetic(from: usize, to: usize) -> Self {
        let mut sub_matches = [0; 20];
        sub_matches[0] = from;
        sub_matches[1] = to;
        Self { sub_matches }
    }

    pub fn str_match_text(&self, s: &str) -> String {
        let (a, b) = self.loc();
        s.chars().skip(a).take(b - a + 1).collect()
    }

    pub fn str_submatch_text(&self, n: usize, s: &str) -> Option<String> {
        let (a, b) = self.sub_loc(n)?;
        Some(s.chars().skip(a).take(b - a + 1).collect())
    }

    pub fn rope_match_text<'a>(&self, r: &'a Rope) -> RopeSlice<'a> {
        let (a, b) = self.loc();
        r.slice(a..=b)
    }

    pub fn rope_submatch_text<'a>(&self, n: usize, r: &'a Rope) -> Option<RopeSlice<'a>> {
        let (a, b) = self.sub_loc(n)?;
        Some(r.slice(a..=b))
    }

    pub fn loc(&self) -> (usize, usize) {
        (self.sub_matches[0], self.sub_matches[1])
    }

    pub fn sub_loc(&self, n: usize) -> Option<(usize, usize)> {
        if n > 9 {
            return None;
        }
        let (start, end) = (self.sub_matches[2 * n], self.sub_matches[2 * n + 1]);
        if n > 0 && start == 0 && end == 0 {
            return None;
        }

        Some((start, end))
    }
}

pub trait IndexedChars {
    type I: Iterator<Item = (usize, char)>;
    fn iter_from(&self, from: usize) -> Option<Self::I>;
}

impl<'a> IndexedChars for &'a str {
    type I = Skip<Enumerate<Chars<'a>>>;

    fn iter_from(&self, from: usize) -> Option<Self::I> {
        if from >= self.len() {
            None
        } else {
            Some(self.chars().enumerate().skip(from))
        }
    }
}

impl<'a> IndexedChars for &'a Rope {
    type I = IdxRopeChars<'a>;

    fn iter_from(&self, from: usize) -> Option<Self::I> {
        if from >= self.len_chars() {
            None
        } else {
            Some(IdxRopeChars::new(self, from, self.len_chars()))
        }
    }
}

/// An iterator over sequential, non overlapping matches of a Regex
/// against a given input
pub struct MatchIter<'a, I>
where
    I: IndexedChars,
{
    pub(super) it: I,
    pub(super) r: &'a mut Regex,
    pub(super) from: usize,
}

impl<'a, I> Iterator for MatchIter<'a, I>
where
    I: IndexedChars,
{
    type Item = Match;

    fn next(&mut self) -> Option<Self::Item> {
        let m = self
            .r
            .match_iter(&mut self.it.iter_from(self.from)?, self.from)?;
        (_, self.from) = m.loc();
        self.from += 1;

        Some(m)
    }
}
