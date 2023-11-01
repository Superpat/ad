//! Sam style language for running edit commands using structural regular expressions
use crate::{
    regex::{self, Match},
    util::parse_num,
};
use ropey::Rope;
use std::{
    cmp::{min, Ordering},
    io::Write,
    iter::Peekable,
    str::Chars,
};

mod expr;
mod stream;

use expr::Expr;
pub use stream::{CachedStdin, IterableStream};

/// Variable usable in templates for injecting the current filename.
/// (Following the naming convention used in Awk)
const FNAME_VAR: &str = "$FILENAME";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    EmptyExpressionGroup,
    EmptyExpressionGroupBranch,
    EmptyProgram,
    Eof,
    InvalidRegex(regex::Error),
    InvalidSubstitution(usize),
    MissingAction,
    MissingDelimiter(&'static str),
    UnclosedDelimiter(&'static str, char),
    UnclosedExpressionGroup,
    UnclosedExpressionGroupBranch,
    UnexpectedCharacter(char),
}

impl From<regex::Error> for Error {
    fn from(err: regex::Error) -> Self {
        Error::InvalidRegex(err)
    }
}

/// A parsed and compiled program that can be executed against an input
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    initial_dot: InitialDot,
    exprs: Vec<Expr>,
}

impl Program {
    /// Execute this program against a given IterableStream
    pub fn execute<S, W>(
        &mut self,
        stream: &mut S,
        fname: &str,
        out: &mut W,
    ) -> Result<(usize, usize), Error>
    where
        S: IterableStream,
        W: Write,
    {
        let (from, to) = match self.initial_dot {
            InitialDot::Full => stream.map_initial_dot(0, None),
            InitialDot::Start(f) => stream.map_initial_dot(f, None),
            InitialDot::Range(f, t) => stream.map_initial_dot(f, Some(t)),
            InitialDot::Current => stream.current_dot(),
        };

        let (from, to) = if !self.exprs.is_empty() {
            let initial = Match::synthetic(from, to);
            self.step(stream, initial, 0, fname, out)?
        } else {
            (from, to)
        };

        let ix_max = stream.len_chars() - 1;

        Ok((min(from, ix_max), min(to, ix_max)))
    }

    fn step<S, W>(
        &mut self,
        stream: &mut S,
        m: Match,
        pc: usize,
        fname: &str,
        out: &mut W,
    ) -> Result<(usize, usize), Error>
    where
        S: IterableStream,
        W: Write,
    {
        let (mut from, mut to) = m.loc();

        match self.exprs[pc].clone() {
            Expr::Group(g) => {
                let mut res = (from, to);
                for exprs in g {
                    let mut p = Program {
                        initial_dot: InitialDot::Range(from, to),
                        exprs: exprs.clone(),
                    };
                    res = p.step(stream, m, 0, fname, out)?;
                }
                Ok(res)
            }

            Expr::LoopMatches(mut re) => loop {
                match re.match_iter(&mut stream.iter_between(from, to), from) {
                    Some(m) => {
                        let cur_len = stream.len_chars();
                        (_, from) = self.step(stream, m, pc + 1, fname, out)?;
                        let new_len = stream.len_chars();
                        from += 1;

                        match new_len.cmp(&cur_len) {
                            Ordering::Greater => to += new_len - cur_len,
                            Ordering::Less => to -= cur_len - new_len,
                            _ => (),
                        }

                        if from > to {
                            return Ok((from, to));
                        }
                    }
                    None => return Ok((from, to)),
                }
            },

            Expr::LoopBetweenMatches(mut re) => loop {
                match re.match_iter(&mut stream.iter_between(from, to), from) {
                    Some(m) => {
                        let (initial_to, new_from) = m.loc();
                        if initial_to == 0 {
                            // skip matches of the null string at the start of input
                            from = new_from + 1;
                            continue;
                        }

                        let m = Match::synthetic(from, initial_to - 1);
                        let cur_len = stream.len_chars();
                        (_, _) = self.step(stream, m, pc + 1, fname, out)?;
                        let new_len = stream.len_chars();

                        match new_len.cmp(&cur_len) {
                            Ordering::Greater => to += new_len - cur_len,
                            Ordering::Less => to -= cur_len - new_len,
                            _ => (),
                        }

                        from = new_from + new_len - cur_len + 1;

                        if from > to {
                            return Ok((from, to));
                        }
                    }
                    None => return Ok((from, to)),
                }
            },

            Expr::IfContains(mut re) => {
                if re.matches_iter(&mut stream.iter_between(from, to), from) {
                    self.step(stream, m, pc + 1, fname, out)
                } else {
                    Ok((from, to))
                }
            }

            Expr::IfNotContains(mut re) => {
                if !re.matches_iter(&mut stream.iter_between(from, to), from) {
                    self.step(stream, m, pc + 1, fname, out)
                } else {
                    Ok((from, to))
                }
            }

            Expr::Print(pat) => {
                let s = template_match(&pat, m, stream.contents(), fname)?;
                writeln!(out, "{s}").expect("to be able to write");
                Ok((from, to))
            }

            Expr::Insert(pat) => {
                let s = template_match(&pat, m, stream.contents(), fname)?;
                stream.insert(from, &s);
                Ok((from, to + s.chars().count()))
            }

            Expr::Append(pat) => {
                let s = template_match(&pat, m, stream.contents(), fname)?;
                stream.insert(to + 1, &s);
                Ok((from, to + s.chars().count()))
            }

            Expr::Change(pat) => {
                let s = template_match(&pat, m, stream.contents(), fname)?;
                stream.remove(from, to);
                stream.insert(from, &s);
                Ok((from, from + s.chars().count()))
            }

            Expr::Delete => {
                stream.remove(from, to);
                Ok((from, from))
            }

            Expr::Sub(mut re, pat) => {
                match re.match_iter(&mut stream.iter_between(from, to), from) {
                    Some(m) => {
                        let (mfrom, mto) = m.loc();
                        let s = template_match(&pat, m, stream.contents(), fname)?;
                        stream.remove(mfrom, mto);
                        stream.insert(mfrom, &s);
                        Ok((from, to - (mto - mfrom + 1) + s.chars().count()))
                    }
                    None => Ok((from, to)),
                }
            }

            Expr::SubAll(mut re, pat) => {
                let mut inner_from = from;
                loop {
                    match re.match_iter(&mut stream.iter_between(inner_from, to), inner_from) {
                        Some(m) => {
                            let cur_len = stream.len_chars();

                            let (mfrom, mto) = m.loc();
                            let s = template_match(&pat, m, stream.contents(), fname)?;
                            stream.remove(mfrom, mto);
                            stream.insert(mfrom, &s);

                            let new_len = stream.len_chars();

                            inner_from = mfrom + s.chars().count();

                            match new_len.cmp(&cur_len) {
                                Ordering::Greater => to += new_len - cur_len,
                                Ordering::Less => to -= cur_len - new_len,
                                _ => (),
                            }

                            if inner_from > to {
                                return Ok((from, to));
                            }
                        }
                        None => return Ok((from, to)),
                    }
                }
            }
        }
    }

    /// Attempt to parse a given program input using a known max dot position
    pub fn try_parse(s: &str) -> Result<Self, Error> {
        let mut exprs = vec![];
        let mut it = s.trim().chars().peekable();

        if it.peek().is_none() {
            return Err(Error::EmptyProgram);
        }

        let initial_dot = parse_initial_dot(&mut it)?;
        consume_whitespace(&mut it);

        loop {
            if it.peek().is_none() {
                break;
            }

            match Expr::try_parse(&mut it) {
                Ok(expr) => {
                    exprs.push(expr);
                    consume_whitespace(&mut it);
                }
                Err(Error::Eof) => break,
                Err(e) => return Err(e),
            }
        }

        if exprs.is_empty() {
            return Ok(Self { initial_dot, exprs });
        }

        validate(&exprs)?;

        Ok(Self { initial_dot, exprs })
    }
}

fn consume_whitespace(it: &mut Peekable<Chars>) {
    loop {
        match it.peek() {
            Some(ch) if ch.is_whitespace() => {
                it.next();
            }
            _ => break,
        }
    }
}

fn validate(exprs: &Vec<Expr>) -> Result<(), Error> {
    use Expr::*;

    if exprs.is_empty() {
        return Err(Error::EmptyProgram);
    }

    // Groups branches must be valid sub-programs
    for e in exprs.iter() {
        if let Group(branches) = e {
            for branch in branches.iter() {
                validate(branch)?;
            }
        }
    }

    // Must end with an action
    if !matches!(
        exprs[exprs.len() - 1],
        Group(_) | Insert(_) | Append(_) | Change(_) | Sub(_, _) | SubAll(_, _) | Print(_) | Delete
    ) {
        return Err(Error::MissingAction);
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InitialDot {
    Full,
    Current,
    Start(usize),
    Range(usize, usize),
}

fn parse_initial_dot(it: &mut Peekable<Chars>) -> Result<InitialDot, Error> {
    match it.peek() {
        Some(',') => {
            it.next();
            Ok(InitialDot::Full)
        }

        Some('.') => {
            it.next();
            Ok(InitialDot::Current)
        }

        // n,m | n,
        Some(&c) if c.is_ascii_digit() => {
            it.next();
            let n = parse_num(c, it);
            match it.next() {
                Some(',') => match it.next() {
                    Some(c) if c.is_ascii_digit() => {
                        let m = parse_num(c, it);
                        Ok(InitialDot::Range(n, m))
                    }
                    Some(' ') | None => Ok(InitialDot::Start(n)),
                    Some(ch) => Err(Error::UnexpectedCharacter(ch)),
                },
                Some(ch) => Err(Error::UnexpectedCharacter(ch)),
                None => Err(Error::Eof),
            }
        }

        // Allow omitting the initial dot
        _ => Ok(InitialDot::Full),
    }
}

// FIXME: if a previous sub-match replacement injects a valid var name for a subsequent one
// then we end up attempting to template THAT in a later iteration of the loop.
fn template_match(s: &str, m: Match, r: Rope, fname: &str) -> Result<String, Error> {
    let mut output = if s.contains(FNAME_VAR) {
        s.replace(FNAME_VAR, fname)
    } else {
        s.to_string()
    };

    // replace newline and tab escapes with their literal equivalents
    output = output.replace("\\n", "\n").replace("\\t", "\t");

    let vars = ["$0", "$1", "$2", "$3", "$4", "$5", "$6", "$7", "$8", "$9"];
    for (n, var) in vars.iter().enumerate() {
        if !s.contains(var) {
            continue;
        }
        match m.rope_submatch_text(n, &r) {
            Some(sm) => output = output.replace(var, &sm.to_string()),
            None => return Err(Error::InvalidSubstitution(n)),
        }
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regex::Regex;
    use ropey::Rope;
    use simple_test_case::test_case;
    use Expr::*;

    fn re(s: &str) -> Regex {
        Regex::compile(s).unwrap()
    }

    #[test_case(",", InitialDot::Full; "full")]
    #[test_case("5,", InitialDot::Start(5); "from n")]
    #[test_case("50,", InitialDot::Start(50); "from n multi digit")]
    #[test_case("5,9", InitialDot::Range(5, 9); "from n to m")]
    #[test_case("25,90", InitialDot::Range(25, 90); "from n to m multi digit")]
    #[test]
    fn parse_initial_dot_works(s: &str, expected: InitialDot) {
        let dot = parse_initial_dot(&mut s.chars().peekable()).expect("valid input");
        assert_eq!(dot, expected);
    }

    #[test_case(", p/$0/", vec![Print("$0".to_string())]; "print all")]
    #[test_case(", x/^.*$/ s/foo/bar/", vec![LoopMatches(re("^.*$")), Sub(re("foo"), "bar".to_string())]; "simple loop")]
    #[test_case(", x/^.*$/ g/emacs/ d", vec![LoopMatches(re("^.*$")), IfContains(re("emacs")), Delete]; "loop filter")]
    #[test]
    fn parse_program_works(s: &str, expected: Vec<Expr>) {
        let p = Program::try_parse(s).expect("valid input");
        assert_eq!(
            p,
            Program {
                initial_dot: InitialDot::Full,
                exprs: expected
            }
        );
    }

    #[test_case("", Error::EmptyProgram; "empty program")]
    #[test_case(", x/.*/", Error::MissingAction; "missing action")]
    #[test]
    fn parse_program_errors_correctly(s: &str, expected: Error) {
        let res = Program::try_parse(s);
        assert_eq!(res, Err(expected));
    }

    #[test_case(Insert("X".to_string()), "Xfoo foo foo", (0, 11); "insert")]
    #[test_case(Append("X".to_string()), "foo foo fooX", (0, 11); "append")]
    #[test_case(Change("X".to_string()), "X", (0, 1); "change")]
    #[test_case(Delete, "", (0, 0); "delete")]
    #[test_case(Sub(re("oo"), "X".to_string()), "fX foo foo", (0, 9); "sub single")]
    #[test_case(SubAll(re("oo"), "X".to_string()), "fX fX fX", (0, 7); "sub all")]
    #[test]
    fn step_works(expr: Expr, expected: &str, expected_dot: (usize, usize)) {
        let mut prog = Program {
            initial_dot: InitialDot::Full,
            exprs: vec![expr],
        };
        let mut r = Rope::from_str("foo foo foo");
        let dot = prog
            .step(&mut r, Match::synthetic(0, 10), 0, "test", &mut vec![])
            .unwrap();

        assert_eq!(&r.to_string(), expected);
        assert_eq!(dot, expected_dot);
    }

    #[test_case(", x/foo/ p/$0/", "foo│foo│foo"; "x print")]
    #[test_case(", x/foo/ i/X/", "Xfoo│Xfoo│Xfoo"; "x insert")]
    #[test_case(", x/foo/ a/X/", "fooX│fooX│fooX"; "x append")]
    #[test_case(", x/foo/ c/X/", "X│X│X"; "x change")]
    #[test_case(", x/foo/ s/o/X/", "fXo│fXo│fXo"; "x substitute")]
    #[test_case(", x/foo/ s/o/X/g", "fXX│fXX│fXX"; "x substitute all")]
    #[test_case(", y/foo/ p/>$0</", "foo│foo│foo"; "y print")]
    #[test_case(", y/foo/ i/X/", "fooX│fooX│foo"; "y insert")]
    #[test_case(", y/foo/ a/X/", "foo│Xfoo│Xfoo"; "y append")]
    #[test_case(", y/foo/ c/X/", "fooXfooXfoo"; "y change")]
    #[test_case(", s/oo/X/", "fX│foo│foo"; "sub single")]
    #[test_case(", s/\\w+/X/", "X│foo│foo"; "sub word single")]
    #[test_case(", s/oo/X/g", "fX│fX│fX"; "sub all")]
    #[test_case(", s/.*/X/g", "X"; "sub all dot star")]
    #[test_case(", x/oo/ s/.*/X/g", "fX│fX│fX"; "x sub all dot star")]
    #[test]
    fn execute_produces_the_correct_string(s: &str, expected: &str) {
        let mut prog = Program::try_parse(s).unwrap();
        let mut r = Rope::from_str("foo│foo│foo");
        prog.execute(&mut r, "test", &mut vec![]).unwrap();

        assert_eq!(&r.to_string(), expected);
    }

    #[test]
    fn sub_g_is_sugar_for_xc() {
        let mut prog1 = Program::try_parse(", s/oo/X/g").unwrap();
        let mut r1 = Rope::from_str("foo│foo│foo");
        prog1.execute(&mut r1, "test", &mut vec![]).unwrap();

        let mut prog2 = Program::try_parse(", x/oo/ c/X/").unwrap();
        let mut r2 = Rope::from_str("foo│foo│foo");
        prog2.execute(&mut r2, "test", &mut vec![]).unwrap();

        assert_eq!(r1.to_string(), r2.to_string());
    }
}
