use crate::buffer::{Buffer, Cur};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(super) enum Kind {
    Insert,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Txt {
    Char(char),
    String(String),
}

impl Txt {
    fn append(&mut self, t: Txt) {
        let push = |buf: &mut String| match t {
            Txt::Char(c) => buf.push(c),
            Txt::String(s) => buf.push_str(&s),
        };

        match self {
            Txt::Char(c) => {
                let mut buf = c.to_string();
                push(&mut buf);
                *self = Txt::String(buf);
            }
            Txt::String(s) => push(s),
        };
    }

    fn prepend(&mut self, t: Txt) {
        let insert = |buf: &mut String| match t {
            Txt::Char(c) => buf.insert(0, c),
            Txt::String(s) => buf.insert_str(0, &s),
        };

        match self {
            Txt::Char(c) => {
                let mut buf = c.to_string();
                insert(&mut buf);
                *self = Txt::String(buf);
            }
            Txt::String(s) => insert(s),
        };
    }
}

/// An Edit represents an atomic change to the state of a Buffer that can be rolled
/// back if needed. Sequential edits to the Buffer are compressed from char based
/// to String based where possible in order to simplify undo state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Edit {
    pub(super) kind: Kind,
    pub(super) cur: Cur,
    pub(super) txt: Txt,
}

impl Edit {
    fn into_undo(mut self) -> Self {
        self.kind = match self.kind {
            Kind::Insert => Kind::Delete,
            Kind::Delete => Kind::Insert,
        };

        self
    }

    pub fn string_repr(&self, b: &Buffer) -> String {
        let indicator = if self.kind == Kind::Insert { "I" } else { "D" };
        let addr = self.cur.as_string_addr(b);

        match &self.txt {
            Txt::Char(c) => format!("{indicator} {addr} '{}'", c),
            Txt::String(s) => format!("{indicator} {addr} '{}'", s.replace('\n', "\\n")),
        }
    }

    fn try_combine(&mut self, e: Edit) -> Option<Edit> {
        match (self.kind, e.kind) {
            (Kind::Insert, Kind::Insert) => self.try_extend_insert(e),
            (Kind::Delete, Kind::Delete) => self.try_extend_delete(e),

            // There are other cases that _could_ be handled here where the kind is still matching
            // and the characters being inserted/deleted are still part of a continuous region of
            // the buffer, but for now this is sufficent for the common case of the user typing
            // without explicitly moving the cursor.
            _ => Some(e),
        }
    }

    fn try_extend_insert(&mut self, e: Edit) -> Option<Edit> {
        if e.cur == self.cur {
            self.txt.prepend(e.txt);
            None
        } else {
            match &self.txt {
                Txt::Char(_) if e.cur.idx == self.cur.idx + 1 => {
                    self.txt.append(e.txt);
                    None
                }
                Txt::String(s) if e.cur.idx == self.cur.idx + s.len() => {
                    self.txt.append(e.txt);
                    None
                }
                _ => Some(e),
            }
        }
    }

    fn try_extend_delete(&mut self, e: Edit) -> Option<Edit> {
        if e.cur == self.cur {
            self.txt.append(e.txt);
            None
        } else {
            match &e.txt {
                Txt::Char(_) if e.cur.idx + 1 == self.cur.idx => {
                    self.txt.prepend(e.txt);
                    self.cur = e.cur;
                    None
                }
                Txt::String(s) if e.cur.idx + s.len() == self.cur.idx => {
                    self.txt.prepend(e.txt);
                    self.cur = e.cur;
                    None
                }
                _ => Some(e),
            }
        }
    }
}

pub type Transaction = Vec<Edit>;

/// An edit log represents the currently undo-able state changes made to a Buffer.
///
/// The log can be unwound, restoring the buffer to a previous state, and rewound as long
/// as no new edits have been made to the buffer (i.e. it is a flat timeline not a tree).
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub(crate) struct EditLog {
    pub(super) edits: Vec<Transaction>,
    pub(super) undone_edits: Vec<Transaction>,
    pub(super) paused: bool,
}

impl EditLog {
    pub(crate) fn clear(&mut self) {
        self.edits.clear();
        self.undone_edits.clear();
        self.paused = false;
    }

    pub(crate) fn debug_edits(&self, b: &Buffer) -> Vec<String> {
        self.edits
            .iter()
            .flat_map(|t| t.iter().map(|e| e.string_repr(b)))
            .collect()
    }

    pub(crate) fn undo(&mut self) -> Option<Transaction> {
        let mut t = self.edits.pop()?;
        while t.is_empty() {
            t = self.edits.pop()?;
        }

        self.undone_edits.push(t.clone());
        t.reverse();

        Some(t.into_iter().map(|e| e.into_undo()).collect())
    }

    pub(crate) fn redo(&mut self) -> Option<Transaction> {
        let mut t = self.undone_edits.pop()?;
        while t.is_empty() {
            t = self.edits.pop()?;
        }

        self.edits.push(t.clone());

        Some(t)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    /// Record a single character being inserted at the given cursor position
    pub(crate) fn insert_char(&mut self, cur: Cur, c: char) {
        if !self.paused {
            self.push(Edit {
                kind: Kind::Insert,
                cur,
                txt: Txt::Char(c),
            });
        }
    }

    /// Record a string being inserted, starting at the given cursor position
    pub(crate) fn insert_string(&mut self, cur: Cur, s: String) {
        if !self.paused {
            self.push(Edit {
                kind: Kind::Insert,
                cur,
                txt: Txt::String(s),
            });
        }
    }

    /// Record a single character being deleted from the given cursor position
    pub(crate) fn delete_char(&mut self, cur: Cur, c: char) {
        if !self.paused {
            self.push(Edit {
                kind: Kind::Delete,
                cur,
                txt: Txt::Char(c),
            });
        }
    }

    /// Record a string being deleted starting at the given cursor position
    pub(crate) fn delete_string(&mut self, cur: Cur, s: String) {
        if !self.paused {
            self.push(Edit {
                kind: Kind::Delete,
                cur,
                txt: Txt::String(s),
            });
        }
    }

    pub(crate) fn new_transaction(&mut self) {
        match self.edits.last() {
            Some(t) if t.is_empty() => return,
            _ => (),
        }
        self.edits.push(Vec::new());
    }

    fn push(&mut self, e: Edit) {
        self.undone_edits.clear();

        if self.edits.is_empty() {
            self.edits.push(vec![e]);
            return;
        }

        let transaction = self.edits.last_mut().unwrap();
        if transaction.is_empty() {
            transaction.push(e);
            return;
        }

        // So long as we have at least one existing edit we can try to extend it
        // by combining it with this new one. If that fails we simply store the
        // new edit as provided.
        if let Some(e) = transaction.last_mut().unwrap().try_combine(e) {
            self.edits.last_mut().unwrap().push(e);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use simple_test_case::test_case;

    pub(crate) fn in_c(idx: usize, c: char) -> Edit {
        Edit {
            kind: Kind::Insert,
            cur: Cur { idx },
            txt: Txt::Char(c),
        }
    }

    pub(crate) fn in_s(idx: usize, s: &str) -> Edit {
        Edit {
            kind: Kind::Insert,
            cur: Cur { idx },
            txt: Txt::String(s.to_string()),
        }
    }

    pub(crate) fn del_c(idx: usize, c: char) -> Edit {
        Edit {
            kind: Kind::Delete,
            cur: Cur { idx },
            txt: Txt::Char(c),
        }
    }

    pub(crate) fn del_s(idx: usize, s: &str) -> Edit {
        Edit {
            kind: Kind::Delete,
            cur: Cur { idx },
            txt: Txt::String(s.to_string()),
        }
    }

    #[test_case(
        vec![in_c(0, 'a'), in_c(1, 'b')],
        &[in_s(0, "ab")];
        "run of characters"
    )]
    #[test_case(
        vec![in_c(0, 'a'), in_c(0, 'b')],
        &[in_s(0, "ba")];
        "run of characters at same cursor"
    )]
    #[test_case(
        vec![in_c(0, 'a'), in_s(1, "bcd")],
        &[in_s(0, "abcd")];
        "char then string"
    )]
    #[test_case(
        vec![in_c(0, 'a'), in_s(0, "bcd")],
        &[in_s(0, "bcda")];
        "char then string at same cursor"
    )]
    #[test_case(
        vec![in_s(0, "ab"), in_s(2, "cd")],
        &[in_s(0, "abcd")];
        "run of strings"
    )]
    #[test_case(
        vec![in_s(0, "ab"), in_s(0, "cd")],
        &[in_s(0, "cdab")];
        "run of strings at same cursor"
    )]
    #[test_case(
        vec![in_s(0, "abc"), in_c(3, 'd')],
        &[in_s(0, "abcd")];
        "string then char"
    )]
    #[test_case(
        vec![in_s(0, "abc"), in_c(0, 'd')],
        &[in_s(0, "dabc")];
        "string then char at same cursor"
    )]
    #[test]
    fn inserts_work(edits: Vec<Edit>, expected: &[Edit]) {
        let mut log = EditLog::default();
        for e in edits {
            log.push(e);
        }

        assert_eq!(&log.edits, &[expected.to_vec()]);
    }

    #[test_case(
        vec![del_c(1, 'b'), del_c(0, 'a')],
        &[del_s(0, "ab")];
        "run of chars"
    )]
    #[test_case(
        vec![del_c(0, 'a'), del_c(0, 'b')],
        &[del_s(0, "ab")];
        "run of characters at same cursor"
    )]
    #[test_case(
        vec![del_c(3, 'd'), del_s(0, "abc")],
        &[del_s(0, "abcd")];
        "char then string"
    )]
    #[test_case(
        vec![del_c(0, 'a'), del_s(0, "bcd")],
        &[del_s(0, "abcd")];
        "char then string at same cursor"
    )]
    #[test_case(
        vec![del_s(2, "cde"), del_s(0, "ab")],
        &[del_s(0, "abcde")];
        "run of strings"
    )]
    #[test_case(
        vec![del_s(0, "abc"), del_s(0, "de")],
        &[del_s(0, "abcde")];
        "run of strings at same cursor"
    )]
    #[test_case(
        vec![del_s(0, "abc"), del_c(0, 'd')],
        &[del_s(0, "abcd")];
        "string then char"
    )]
    #[test_case(
        vec![del_s(0, "abc"), del_c(0, 'd')],
        &[del_s(0, "abcd")];
        "string then char at same cursor"
    )]
    #[test]
    fn delete_work(edits: Vec<Edit>, expected: &[Edit]) {
        let mut log = EditLog::default();
        for e in edits {
            log.push(e);
        }

        assert_eq!(&log.edits, &[expected.to_vec()]);
    }
}
