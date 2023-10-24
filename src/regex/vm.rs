//! Virtual machine based implementation based on the instruction set described
//! in Russ Cox's second article in the series and the source of plan9 Sam:
//!   https://swtch.com/~rsc/regexp/regexp2.html
//!   https://github.com/sminez/plan9port/blob/master/src/cmd/sam/regexp.c
//!
//! The compilation step used is custom (rather than using a YACC parser).
//!
//! We make use of pre-allocated buffers for the Thread lists and track the
//! index we are up to per-iteration as this results in roughly a 100x speed
//! up from not having to allocate and free inside of the main loop.
use super::{re_to_postfix, CharClass, Error, Pfix};
use std::{collections::BTreeSet, mem::take};

pub struct Regex {
    /// The compiled instructions for running the VM
    prog: Prog,
    /// Pre-allocated Thread list
    clist: Vec<usize>,
    /// Pre-allocated Thread list
    nlist: Vec<usize>,
    /// Monotonically increasing index used to dedup Threads
    /// Will overflow at some point if a given regex is used a VERY large number of times
    gen: usize,
    /// Index into the current Thread list
    p: usize,
}

impl Regex {
    pub fn compile(re: &str) -> Result<Self, Error> {
        let pfix = re_to_postfix(re)?;
        let ops = optimise(compile(pfix));
        let prog: Prog = ops.into_iter().map(|op| Inst { op, gen: 0 }).collect();

        let clist = vec![0; prog.len()];
        let nlist = vec![0; prog.len()];

        Ok(Self {
            prog,
            clist,
            nlist,
            gen: 1,
            p: 0,
        })
    }

    pub fn matches_str(&mut self, input: &str) -> bool {
        self.matches_iter(input.chars().enumerate())
    }

    pub fn matches_iter<I>(&mut self, input: I) -> bool
    where
        I: Iterator<Item = (usize, char)>,
    {
        let mut clist = take(&mut self.clist);
        let mut nlist = take(&mut self.nlist);
        self.p = 0;

        self.add_thread(&mut clist, 0);
        self.gen += 1;
        let mut n = self.p;
        let mut matched = false;

        println!(
            "PROG: {:?}",
            self.prog.iter().map(|i| i.op.clone()).collect::<Vec<_>>()
        );
        for (_, ch) in input {
            println!("CHAR: {ch} :: CLIST: {clist:?}");
            for &tpc in clist.iter().take(n) {
                println!("OP: {:?}", self.prog[tpc].op);
                match &self.prog[tpc].op {
                    Op::Char(c) if *c == ch => self.add_thread(&mut nlist, tpc + 1),
                    Op::Class(cls) if cls.matches_char(ch) => self.add_thread(&mut nlist, tpc + 1),
                    Op::Any if ch != '\n' => self.add_thread(&mut nlist, tpc + 1),
                    Op::TrueAny => self.add_thread(&mut nlist, tpc + 1),

                    Op::Match => {
                        matched = true;
                        break;
                    }

                    // Save, Jump & Split are handled in add_thread.
                    // Non-matching comparison ops result in that thread dying.
                    _ => (),
                }
            }

            (clist, nlist) = (nlist, clist);

            if self.p == 0 {
                break;
            }

            self.gen += 1;
            n = self.p;
            self.p = 0;
        }

        self.clist = clist;
        self.nlist = nlist;
        matched || self.clist.iter().any(|&tpc| self.prog[tpc].op == Op::Match)
    }

    fn add_thread(&mut self, lst: &mut [usize], pc: usize) {
        if self.prog[pc].gen == self.gen {
            return;
        }
        self.prog[pc].gen = self.gen;

        if let Op::Jump(l1) = self.prog[pc].op {
            self.add_thread(lst, l1);
        } else if let Op::Split(l1, l2) = self.prog[pc].op {
            self.add_thread(lst, l1);
            self.add_thread(lst, l2);
        } else if let Op::Save(_) = self.prog[pc].op {
            // TODO: impl submatch captures
            self.add_thread(lst, pc + 1);
        } else {
            lst[self.p] = pc;
            self.p += 1;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Op {
    // Comparison ops
    Char(char),
    Class(CharClass),
    Any,
    TrueAny,
    // Control ops
    Split(usize, usize),
    Jump(usize),
    Save(usize),
    Match,
}

impl Op {
    fn is_comp(&self) -> bool {
        !matches!(self, Op::Split(_, _) | Op::Jump(_) | Op::Match)
    }

    fn inc(&mut self, i: usize) {
        match self {
            Op::Jump(j) if *j >= i => *j += 1,
            Op::Split(l1, l2) => {
                if *l1 >= i {
                    *l1 += 1;
                }
                if *l2 >= i {
                    *l2 += 1;
                }
            }
            _ => (),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Inst {
    op: Op,
    gen: usize,
}

type Prog = Vec<Inst>;

fn compile(pfix: Vec<Pfix>) -> Vec<Op> {
    let mut prog: Vec<Op> = Vec::with_capacity(pfix.len());
    let mut expr_offsets: Vec<usize> = Vec::with_capacity(pfix.len());

    macro_rules! push {
        ($op:expr) => {{
            expr_offsets.push(prog.len());
            prog.push($op);
        }};
        (@expr $exp:expr) => {{
            expr_offsets.push(prog.len());
            prog.append(&mut $exp);
        }};
        (@save $s:expr) => {{
            if $s % 2 == 0 {
                expr_offsets.push(prog.len());
            } else {
                let ix = prog
                    .iter()
                    .position(|op| op == &Op::Save($s - 1))
                    .expect("to have save start");
                expr_offsets.truncate(ix + 1);
            }
            prog.push(Op::Save($s));
        }};
    }

    macro_rules! pop {
        () => {{
            let ix = expr_offsets.pop().unwrap();
            prog.split_off(ix)
        }};
    }

    for p in pfix.into_iter() {
        match p {
            Pfix::Class(cls) => push!(Op::Class(cls)),
            Pfix::Char(ch) => push!(Op::Char(ch)),
            Pfix::TrueAny => push!(Op::TrueAny),
            Pfix::Any => push!(Op::Any),

            Pfix::Concat => {
                expr_offsets.pop();
            }

            Pfix::Alt => {
                let mut e2 = pop!();
                let mut e1 = pop!();
                let ix = prog.len(); // index of the split we are inserting

                push!(Op::Split(ix + 1, ix + 2 + e1.len()));
                e1.iter_mut().for_each(|op| op.inc(ix));
                e2.iter_mut().for_each(|op| op.inc(ix));
                push!(@expr e1);

                let ix2 = prog.len();
                push!(Op::Jump(ix2 + 1 + e2.len()));
                e2.iter_mut().for_each(|op| op.inc(ix2));
                push!(@expr e2);
            }

            Pfix::Plus => {
                let ix = *expr_offsets.last().unwrap();
                push!(Op::Split(ix, prog.len() + 1));
            }

            Pfix::Quest => {
                let mut e = pop!();
                let ix = prog.len(); // index of the split we are inserting

                push!(Op::Split(ix + 1, ix + 1 + e.len()));
                e.iter_mut().for_each(|op| op.inc(ix));
                push!(@expr e);
            }

            Pfix::Star => {
                let mut e = pop!();
                let ix = prog.len(); // index of the split we are inserting

                push!(Op::Split(ix + 1, ix + 2 + e.len()));
                e.iter_mut().for_each(|op| op.inc(ix));
                push!(@expr e);
                push!(Op::Jump(ix))
            }

            Pfix::Save(s) => push!(@save s),
        }
    }

    prog.push(Op::Match);

    prog
}

fn optimise(mut ops: Vec<Op>) -> Vec<Op> {
    let mut optimising = true;

    while optimising {
        optimising = false;
        for i in 0..ops.len() {
            optimising |= inline_jumps(&mut ops, i);
        }
    }

    strip_unreachable_instructions(&mut ops);

    ops
}

// - Chained jumps or jumps to splits can be inlined
// - Split to jump can be inlined
// - Jump to Match is just Match
// - Split to Match is Match if both branches are Match,
//   otherwise there could be a longer match available
//   on the non-Match branch so we keep the split
#[inline]
fn inline_jumps(ops: &mut [Op], i: usize) -> bool {
    if let Op::Jump(j) = ops[i] {
        if let Op::Jump(l1) = ops[j] {
            ops[i] = Op::Jump(l1);
        } else if let Op::Split(l1, l2) = ops[j] {
            ops[i] = Op::Split(l1, l2);
        } else if let Op::Match = ops[j] {
            ops[i] = Op::Match;
        } else {
            return false;
        }
        return true;
    } else if let Op::Split(s1, s2) = ops[i] {
        if ops[s1] == Op::Match && ops[s2] == Op::Match {
            ops[i] = Op::Match;
            return true;
        }

        let new_s1 = if let Op::Jump(j1) = ops[s1] { j1 } else { s1 };
        let new_s2 = if let Op::Jump(j2) = ops[s2] { j2 } else { s2 };
        if new_s1 != s1 || new_s2 != s2 {
            ops[i] = Op::Split(new_s1, new_s2);
            return true;
        }
    }

    false
}

// An instruction is unreachable if:
// - it doesn't follow a comparison instruction (pc wouldn't advance to it)
// - nothing now jumps or splits to it
fn strip_unreachable_instructions(ops: &mut Vec<Op>) {
    let mut to_from: Vec<(usize, usize)> = Vec::with_capacity(ops.len());
    let mut jumps = BTreeSet::new();

    for (i, op) in ops.iter().enumerate() {
        match op {
            Op::Jump(j) => {
                jumps.insert(*j);
                to_from.push((*j, i));
            }
            Op::Split(l1, l2) => {
                jumps.extend([*l1, *l2]);
                to_from.push((*l1, i));
                to_from.push((*l2, i));
            }
            _ => (),
        }
    }

    for i in (1..ops.len() - 1).rev() {
        if ops[i - 1].is_comp() || jumps.contains(&i) {
            continue;
        }

        for &(to, from) in to_from.iter() {
            if to > i {
                match &mut ops[from] {
                    Op::Jump(x) => *x -= 1,
                    Op::Split(x, _) if *x > i => *x -= 1,
                    Op::Split(_, x) if *x > i => *x -= 1,
                    _ => (),
                }
            }
        }
        ops.remove(i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simple_test_case::test_case;

    fn sp(l1: usize, l2: usize) -> Op {
        Op::Split(l1, l2)
    }

    fn jmp(l1: usize) -> Op {
        Op::Jump(l1)
    }

    fn c(ch: char) -> Op {
        Op::Char(ch)
    }

    fn sv(s: usize) -> Op {
        Op::Save(s)
    }

    #[test_case("abc", &[c('a'), c('b'), c('c'), Op::Match]; "lit only")]
    #[test_case("a|b", &[sp(1, 3), c('a'), jmp(4), c('b'), Op::Match]; "single char alt")]
    #[test_case("ab(c|d)", &[c('a'), c('b'), sv(2), sp(4, 6), c('c'), jmp(7), c('d'), sv(3), Op::Match]; "lits then alt")]
    #[test_case("ab+a", &[c('a'), c('b'), sp(1, 3), c('a'), Op::Match]; "plus for single lit")]
    #[test_case("ab?a", &[c('a'), sp(2, 3), c('b'), c('a'), Op::Match]; "quest for single lit")]
    #[test_case("ab*a", &[c('a'), sp(2, 4), c('b'), jmp(1), c('a'), Op::Match]; "star for single lit")]
    #[test_case("a(bb)+a", &[c('a'), sv(2), c('b'), c('b'), sv(3), sp(2, 6), c('a'), Op::Match]; "rep of cat")]
    #[test_case("ba*", &[c('b'), sp(2, 4), c('a'), jmp(1), Op::Match]; "trailing star")]
    #[test_case("b?a", &[sp(1, 2), c('b'), c('a'), Op::Match]; "first lit is optional")]
    #[test_case("(a*)", &[sv(2), sp(2, 4), c('a'), jmp(1), sv(3), Op::Match]; "star")]
    #[test_case("(a*)*", &[sp(1, 7), sv(2), sp(3, 5), c('a'), jmp(2), sv(3), jmp(0), Op::Match]; "star star")]
    #[test]
    fn opcode_compile_works(re: &str, expected: &[Op]) {
        let prog = compile(re_to_postfix(re).unwrap());
        assert_eq!(&prog, expected);
    }

    #[test_case("a|b", &[sp(1, 3), c('a'), Op::Match, c('b'), Op::Match]; "single char alt")]
    #[test_case("ab(c|d)", &[c('a'), c('b'), sv(2), sp(4, 6), c('c'), jmp(7), c('d'), sv(3), Op::Match]; "lits then alt")]
    #[test_case("ab*a", &[c('a'), sp(2, 4), c('b'), sp(2, 4), c('a'), Op::Match]; "star for single lit")]
    #[test_case("ba*", &[c('b'), sp(2, 4), c('a'), sp(2, 4), Op::Match]; "trailing star")]
    #[test_case("(a*)*", &[sp(1, 7), sv(2), sp(3, 5), c('a'), sp(3, 5), sv(3), sp(1, 7), Op::Match]; "star star")]
    #[test]
    fn opcode_optimise_works(re: &str, expected: &[Op]) {
        let prog = optimise(compile(re_to_postfix(re).unwrap()));
        assert_eq!(&prog, expected);
    }
}
