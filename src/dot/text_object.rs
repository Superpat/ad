//! Vim style text objects
use crate::{
    buffer::Buffer,
    dot::{
        find::{expand, find_backward_start, find_forward_end, Find},
        Cur, Dot, Range,
    },
    key::Arrow,
};

/// A vim-like text object which can be used to manipulate the current Dot in a Buffer
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum TextObject {
    Arr(Arrow),
    BufferEnd,
    BufferStart,
    Character,
    // Delimited(char, char),
    // FindChar(char),
    Line,
    LineEnd,
    LineStart,
    Paragraph,
    Word,
}

impl TextObject {
    pub fn set_dot(&self, b: &mut Buffer) {
        use TextObject::*;

        let dot = match self {
            Arr(arr) => b.dot.active_cur().arr(*arr, b).into(),
            BufferEnd => Cur::buffer_end(b).into(),
            BufferStart => Cur::buffer_start().into(),
            Character => b.dot.active_cur().arr(Arrow::Right, b).into(),
            LineEnd => b.dot.active_cur().move_to_line_end(b).into(),
            LineStart => b.dot.active_cur().move_to_line_start(b).into(),
            Line => Dot::from(
                b.dot
                    .as_range()
                    .extend_to_line_start(b)
                    .extend_to_line_end(b),
            )
            .collapse_null_range(),
            Paragraph => expand(&FindParagraph::Fwd, b.dot, b),
            Word => expand(&FindWord::Fwd, b.dot, b),
        };

        b.dot = dot;
    }

    pub fn extend_dot_forward(&self, b: &mut Buffer) {
        use TextObject::*;

        let Range {
            mut start,
            mut end,
            start_active,
        } = b.dot.as_range();

        (start, end) = match (self, start_active) {
            (Arr(arr), _) => (start, end.arr(*arr, b)),
            (BufferEnd, true) => (end, Cur::buffer_end(b)),
            (BufferEnd, false) => (start, Cur::buffer_end(b)),
            (BufferStart, _) => return, // Can't move forward to the buffer start
            (Character, true) => (start.arr_w_count(Arrow::Right, 1, b), end),
            (Character, false) => (start, end.arr_w_count(Arrow::Right, 1, b)),
            (Line, true) => (start.arr_w_count(Arrow::Down, 1, b), end),
            (Line, false) => (start, end.arr_w_count(Arrow::Down, 1, b)),
            (LineEnd, true) => (start.move_to_line_end(b), end),
            (LineEnd, false) => (start, end.move_to_line_end(b)),
            (LineStart, true) => (start.move_to_line_start(b), end),
            (LineStart, false) => (start, end.move_to_line_start(b)),
            (Paragraph, true) => (find_forward_end(&FindParagraph::Fwd, start, b), end),
            (Paragraph, false) => (start, find_forward_end(&FindParagraph::Fwd, end, b)),
            (Word, true) => (find_forward_end(&FindWord::Fwd, start, b), end),
            (Word, false) => (start, find_forward_end(&FindWord::Fwd, end, b)),
        };

        b.dot = Dot::from(Range::from_cursors(start, end, start_active)).collapse_null_range();
    }

    pub fn extend_dot_backward(&self, b: &mut Buffer) {
        use TextObject::*;

        let Range {
            mut start,
            mut end,
            start_active,
        } = b.dot.as_range();

        (start, end) = match (self, start_active) {
            (Arr(arr), _) => (start.arr(arr.flip(), b), end),
            (BufferEnd, _) => return, // Can't move back to the buffer end
            (BufferStart, true) => (Cur::buffer_start(), end),
            (BufferStart, false) => (Cur::buffer_start(), start),
            (Character, true) => (start.arr_w_count(Arrow::Left, 1, b), end),
            (Character, false) => (start, end.arr_w_count(Arrow::Left, 1, b)),
            (Line, true) => (start.arr_w_count(Arrow::Up, 1, b), end),
            (Line, false) => (start, end.arr_w_count(Arrow::Up, 1, b)),
            (LineEnd, true) => (start.move_to_line_end(b), end),
            (LineEnd, false) => (start, end.move_to_line_end(b)),
            (LineStart, true) => (start.move_to_line_start(b), end),
            (LineStart, false) => (start, end.move_to_line_start(b)),
            (Paragraph, true) => (find_backward_start(&FindParagraph::Fwd, start, b), end),
            (Paragraph, false) => (start, find_backward_start(&FindParagraph::Fwd, end, b)),
            (Word, true) => (find_backward_start(&FindWord::Fwd, start, b), end),
            (Word, false) => (start, find_backward_start(&FindWord::Fwd, end, b)),
        };

        b.dot = Dot::from(Range::from_cursors(start, end, start_active)).collapse_null_range();
    }
}

enum FindParagraph {
    Fwd,
    Bck,
}

impl Find for FindParagraph {
    type Reversed = FindParagraph;

    fn reversed(&self) -> Self::Reversed {
        match self {
            Self::Fwd => Self::Bck,
            Self::Bck => Self::Fwd,
        }
    }

    fn try_find<I>(&self, it: I) -> Option<(usize, usize)>
    where
        I: Iterator<Item = (usize, char)>,
    {
        let mut prev_was_newline = false;
        let mut pos = 0;

        for (i, ch) in it {
            match ch {
                '\n' if prev_was_newline => {
                    return match self {
                        Self::Fwd => Some((i, i)),
                        Self::Bck => Some((i + 1, i + 1)),
                    }
                }
                '\n' => prev_was_newline = true,
                _ => prev_was_newline = false,
            }
            pos = i;
        }

        Some((pos, pos))
    }
}

/// Searching forward targets the end of words and searching back targets the start
enum FindWord {
    Fwd,
    Bck,
}

impl Find for FindWord {
    type Reversed = FindWord;

    fn reversed(&self) -> Self::Reversed {
        match self {
            Self::Fwd => Self::Bck,
            Self::Bck => Self::Fwd,
        }
    }

    fn try_find<I>(&self, it: I) -> Option<(usize, usize)>
    where
        I: Iterator<Item = (usize, char)>,
    {
        use CharKind::*;

        let mut it = it.peekable();
        let mut prev = CharKind::from(it.peek()?.1);

        // If we are searching forward and are not currently sat on whitespace then we could be
        // on the end of a word which would cause us to stick in place, so we advance a single
        // character and start the search from there.
        if matches!((prev, self), (Word | Punctuation, FindWord::Fwd)) {
            it.next();
            prev = CharKind::from(it.peek()?.1);
        }

        for (i, ch) in it {
            let kind = CharKind::from(ch);
            match (prev, kind) {
                (Word, Punctuation) | (Punctuation, Word) | (Word | Punctuation, Whitespace) => {
                    return match self {
                        Self::Fwd => Some((i - 1, i - 1)),
                        Self::Bck => Some((i + 1, i + 1)),
                    }
                }
                _ => prev = kind,
            }
        }

        None
    }
}

#[derive(Clone, Copy)]
enum CharKind {
    Word,
    Punctuation,
    Whitespace,
}

impl From<char> for CharKind {
    fn from(ch: char) -> Self {
        if ch.is_alphanumeric() || ch == '_' {
            CharKind::Word
        } else if ch.is_whitespace() {
            CharKind::Whitespace
        } else {
            CharKind::Punctuation
        }
    }
}
