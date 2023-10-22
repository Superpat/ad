//! A simple regex engine for operating on character streams and supporting
//! the Sam text editor's structural regular expressions.
//!
//! The implementation of this engine is adapted from the one presented by
//! Russ Cox here:
//!   https://swtch.com/~rsc/regexp/regexp1.html
//!
//! Thompson's original paper on writing a regex engine can be found here:
//!   https://dl.acm.org/doi/pdf/10.1145/363347.363387

// Different impls of the matching algorithm
pub mod dfa;
pub mod vm;

const POSTFIX_BUF_SIZE: usize = 2000;
const POSTFIX_MAX_PARENS: usize = 100;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    EmptyParens,
    EmptyRegex,
    InvalidClass,
    InvalidEscape(char),
    InvalidRepetition,
    ReTooLong,
    TooManyParens,
    UnbalancedAlt,
    UnbalancedParens,
}

// Postfix form notation for building up the compiled state machine
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pfix {
    Char(char),
    Class(CharClass),
    Any,
    TrueAny,
    Concat,
    Alt,
    Quest,
    Star,
    Plus,
}

/// Helper for converting characters to 0 based inicies for looking things up in caches.
const fn char_ix(ch: char) -> usize {
    ((ch as u16) & 0xFF) as usize
}

const fn init_escapes() -> [Option<char>; 256] {
    macro_rules! escape {
        ($escapes:expr, $($ch:expr),+) => {
            $($escapes[char_ix($ch)] = Some($ch);)+
        };
        ($escapes:expr, $($ch:expr => $esc:expr),+) => {
            $($escapes[char_ix($ch)] = Some($esc);)+
        };
    }

    let mut escapes = [None; 256];
    escape!(escapes, '*', '+', '?', '.', '@', '(', ')', '[', ']', '|');
    escape!(escapes, '\\', '\'', '"');
    escape!(escapes, 'n'=>'\n', 'r'=>'\r', 't'=>'\t');

    escapes
}

/// Supported escape sequences
const ESCAPES: [Option<char>; 256] = init_escapes();

fn insert_cats(natom: &mut usize, output: &mut Vec<Pfix>) {
    *natom -= 1;
    while *natom > 0 {
        output.push(Pfix::Concat);
        *natom -= 1;
    }
}

fn insert_alts(nalt: &mut usize, output: &mut Vec<Pfix>) {
    while *nalt > 0 {
        output.push(Pfix::Alt);
        *nalt -= 1;
    }
}

fn push_cat(natom: &mut usize, output: &mut Vec<Pfix>) {
    if *natom > 1 {
        output.push(Pfix::Concat);
        *natom -= 1;
    }
}

fn push_atom(p: Pfix, natom: &mut usize, output: &mut Vec<Pfix>) {
    push_cat(natom, output);
    output.push(p);
    *natom += 1;
}

fn push_rep(p: Pfix, natom: usize, output: &mut Vec<Pfix>) -> Result<(), Error> {
    if natom == 0 {
        return Err(Error::InvalidRepetition);
    }
    output.push(p);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CharClass {
    negated: bool,
    chars: Vec<char>,
    ranges: Vec<(char, char)>,
}

impl CharClass {
    fn try_parse(it: &mut std::str::Chars) -> Result<Self, Error> {
        let mut next = || next_char(it)?.ok_or(Error::InvalidClass);
        let (mut ch, _) = next()?;

        let negated = ch == '^';
        if negated {
            (ch, _) = next()?
        };
        let mut chars = vec![ch];
        let mut ranges = vec![];

        loop {
            let (ch, escaped) = next()?;
            match ch {
                ']' if !escaped => break,

                '-' => {
                    let start = chars.pop().ok_or(Error::InvalidClass)?;
                    let (end, _) = next()?;
                    ranges.push((start, end));
                }

                ch => chars.push(ch),
            }
        }

        Ok(Self {
            negated,
            chars,
            ranges,
        })
    }

    // Negated classes still don't match a newline
    fn matches_char(&self, ch: char) -> bool {
        if self.negated && ch == '\n' {
            return false;
        }

        let res = self.chars.contains(&ch)
            || self
                .ranges
                .iter()
                .any(|&(start, end)| ch >= start && ch <= end);

        if self.negated {
            !res
        } else {
            res
        }
    }
}

fn next_char(it: &mut std::str::Chars) -> Result<Option<(char, bool)>, Error> {
    match it.next() {
        Some('\\') => (),
        Some(ch) => return Ok(Some((ch, false))),
        None => return Ok(None),
    }

    let ch = match it.next() {
        Some(ch) => ch,
        None => return Err(Error::InvalidEscape('\0')),
    };

    match ESCAPES[char_ix(ch)] {
        Some(ch) => Ok(Some((ch, true))),
        None => Err(Error::InvalidEscape(ch)),
    }
}

fn re_to_postfix(re: &str) -> Result<Vec<Pfix>, Error> {
    #[derive(Clone, Copy)]
    struct Paren {
        natom: usize,
        nalt: usize,
    }

    if re.is_empty() {
        return Err(Error::EmptyRegex);
    } else if re.len() > POSTFIX_BUF_SIZE / 2 {
        return Err(Error::ReTooLong);
    }

    let mut output = Vec::with_capacity(POSTFIX_BUF_SIZE);
    let mut paren: [Paren; POSTFIX_MAX_PARENS] = [Paren { natom: 0, nalt: 0 }; POSTFIX_MAX_PARENS];
    let mut natom = 0;
    let mut nalt = 0;
    let mut p = 0;

    let mut it = re.chars();

    while let Some((ch, escaped)) = next_char(&mut it)? {
        if escaped {
            push_atom(Pfix::Char(ch), &mut natom, &mut output);
            continue;
        }

        match ch {
            '(' => {
                if p >= POSTFIX_MAX_PARENS {
                    return Err(Error::TooManyParens);
                }
                push_cat(&mut natom, &mut output);
                paren[p].natom = natom;
                paren[p].nalt = nalt;
                p += 1;
                natom = 0;
                nalt = 0;
            }

            ')' => {
                if p == 0 {
                    return Err(Error::UnbalancedParens);
                } else if natom == 0 {
                    return Err(Error::EmptyParens);
                }

                insert_cats(&mut natom, &mut output);
                insert_alts(&mut nalt, &mut output);

                p -= 1;
                natom = paren[p].natom;
                nalt = paren[p].nalt;
                natom += 1;
            }

            '|' => {
                if natom == 0 {
                    return Err(Error::UnbalancedAlt);
                }

                insert_cats(&mut natom, &mut output);
                nalt += 1;
            }

            '*' => push_rep(Pfix::Star, natom, &mut output)?,
            '+' => push_rep(Pfix::Plus, natom, &mut output)?,
            '?' => push_rep(Pfix::Quest, natom, &mut output)?,

            '[' => {
                let cls = CharClass::try_parse(&mut it)?;
                push_atom(Pfix::Class(cls), &mut natom, &mut output);
            }

            '.' => push_atom(Pfix::Any, &mut natom, &mut output),
            '@' => push_atom(Pfix::TrueAny, &mut natom, &mut output),

            ch => push_atom(Pfix::Char(ch), &mut natom, &mut output),
        }
    }

    if p != 0 {
        return Err(Error::UnbalancedParens);
    }

    insert_cats(&mut natom, &mut output);
    insert_alts(&mut nalt, &mut output);

    Ok(output)
}
