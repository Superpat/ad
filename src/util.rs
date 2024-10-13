//! Utility functions
use crate::editor::built_in_commands;
use std::{
    iter::Peekable,
    path::{Component, Path, PathBuf},
    str::Chars,
};
use tracing::warn;

/// Pull in data from the ad crate itself to auto-generate the docs on the functionality
/// available in the editor.
pub(crate) fn gen_help_docs() -> String {
    let help_template = include_str!("../data/help-template.txt");

    help_template.replace("{{BUILT_IN_COMMANDS}}", &commands_section())
}

fn commands_section() -> String {
    let commands = built_in_commands();
    let mut buf = Vec::with_capacity(commands.len());

    for (cmds, desc) in commands.into_iter() {
        buf.push((cmds.join(" | "), desc));
    }

    let w_max = buf.iter().map(|(s, _)| s.len()).max().unwrap();
    let mut s = String::new();

    for (cmds, desc) in buf.into_iter() {
        s.push_str(&format!("{:width$} -- {desc}\n", cmds, width = w_max));
    }

    s
}

/// Both the base path and p must be absolute paths and base must be a directory
pub(crate) fn relative_path_from(base: &Path, p: &Path) -> PathBuf {
    let mut base_comps = base.components();
    let mut path_comps = p.components();
    let mut comps = vec![];

    loop {
        match (base_comps.next(), path_comps.next()) {
            (None, None) => break,
            (None, Some(path_comp)) => {
                comps.push(path_comp);
                comps.extend(path_comps.by_ref());
                break;
            }
            (_, None) => comps.push(Component::ParentDir),
            (Some(a), Some(b)) if comps.is_empty() && a == b => (),
            (Some(Component::CurDir), Some(b)) => comps.push(b),
            (Some(_), Some(b)) => {
                comps.push(Component::ParentDir);
                for _ in base_comps {
                    comps.push(Component::ParentDir);
                }
                comps.push(b);
                comps.extend(path_comps.by_ref());
                break;
            }
        }
    }

    comps.iter().collect()
}

// returns the parsed number and following character if there was one.
// initial must be a valid ascii digit
pub(crate) fn parse_num(initial: char, it: &mut Peekable<Chars<'_>>) -> usize {
    let mut s = String::from(initial);
    loop {
        match it.peek() {
            Some(ch) if ch.is_ascii_digit() => {
                s.push(it.next().unwrap());
            }
            _ => return s.parse().unwrap(),
        }
    }
}

pub(crate) fn normalize_line_endings(mut s: String) -> String {
    if !s.contains('\r') {
        return s;
    }

    warn!("normalizing \\r characters to \\n");
    s = s.replace("\r\n", "\n");
    s.replace("\r", "\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use simple_test_case::test_case;

    #[test_case("/home/bob/", "/home/bob/ad/src/lib.rs", "ad/src/lib.rs"; "child directory")]
    #[test_case("/home/bob/ad/src", "/home/bob/ad/README.md", "../README.md"; "relative directory")]
    #[test_case("/home/bob/ad", "/usr/local/bin/penrose", "../../../usr/local/bin/penrose"; "only root in common")]
    #[test]
    fn rel_path_works(base: &str, p: &str, expected: &str) {
        let base = PathBuf::from(base);
        let p = PathBuf::from(p);
        let rel = relative_path_from(&base, &p);

        assert_eq!(rel, PathBuf::from(expected));
    }
}
