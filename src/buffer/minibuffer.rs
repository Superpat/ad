//! A transient buffer for handling interactive input from the user without
//! modifying the current buffer state.
use crate::{
    buffer::{Buffer, GapBuffer, TextObject},
    config,
    editor::Editor,
    key::{Arrow, Key},
    util::run_command,
};
use std::{cmp::min, ffi::OsStr, path::Path};
use tracing::trace;

#[derive(Debug, Default)]
pub(crate) struct MiniBufferState<'a> {
    pub(crate) cx: usize,
    pub(crate) cy: usize,
    pub(crate) selected_line_idx: usize,
    pub(crate) prompt_line: &'a str,
    pub(crate) b: Option<&'a Buffer>,
    pub(crate) top: usize,
    pub(crate) bottom: usize,
}

#[allow(dead_code)]
pub(crate) enum MiniBufferSelection {
    Line {
        cy: usize,
        line: String,
        input: String,
    },
    UserInput {
        input: String,
    },
    Cancelled,
}

/// A mini-buffer always has a single line prompt for accepting user input
/// with the rest of the buffer content not being directly editable.
///
/// Conceptually this is along the lines of the window tags in acme (a space
/// to scribble down additional useful information without affecting the
/// files you are editing) crossed with dmenu.
pub(crate) struct MiniBuffer {
    prompt: String,
    initial_lines: Vec<String>,
    b: Buffer,
    max_height: usize,
}

impl MiniBuffer {
    pub fn new(prompt: String, lines: Vec<String>, max_height: usize) -> Self {
        Self {
            prompt,
            initial_lines: lines,
            b: Buffer::new_minibuffer(),
            max_height,
        }
    }

    /// Force the cursor to be a single Cur and ensure that its y offset is in bounds
    fn handle_on_change<F: Fn(&str) -> Option<Vec<String>>>(&mut self, input: &str, on_change: F) {
        if let Some(lines) = (on_change)(input) {
            self.b.txt = GapBuffer::from(lines.join("\n"));
            self.b.dot.clamp_idx(self.b.txt.len_chars());
        };
    }

    pub fn prompt_w_callback<F: Fn(&str) -> Option<Vec<String>>>(
        prompt: &str,
        initial_lines: Vec<String>,
        on_change: F,
        ed: &mut Editor,
    ) -> MiniBufferSelection {
        let offset = prompt.len();
        let (screen_rows, _) = ed.screen_rowcol();
        let mut mb = MiniBuffer::new(
            prompt.to_string(),
            initial_lines,
            config!().minibuffer_lines,
        );
        let mut input = String::new();
        let mut x = 0;
        let mut line_indices: Vec<usize> = Vec::with_capacity(mb.initial_lines.len());

        loop {
            mb.prompt = format!("{prompt}{input}");
            mb.b.txt.clear();
            line_indices.clear();
            let input_fragments: Vec<&str> = input.split_whitespace().collect();
            let mut visible_lines = vec![];

            for (i, line) in mb.initial_lines.iter().enumerate() {
                let matching = input_fragments.iter().all(|f| {
                    if f.chars().all(|c| c.is_lowercase()) {
                        line.to_lowercase().contains(f)
                    } else {
                        line.contains(f)
                    }
                });

                if matching {
                    visible_lines.push(line.clone());
                    line_indices.push(i);
                }
            }

            mb.b.txt = GapBuffer::from(visible_lines.join("\n"));
            mb.b.dot.clamp_idx(mb.b.txt.len_chars());

            let n_visible_lines = min(visible_lines.len(), mb.max_height);
            let (y, _) = mb.b.dot.active_cur().as_yx(&mb.b);

            let (selected_line_idx, top, bottom, b) = if n_visible_lines == 0 {
                (0, 0, 0, None)
            } else if y >= n_visible_lines {
                let lower = y.saturating_sub(n_visible_lines) + 1;
                (y, lower, y, Some(&mb.b))
            } else {
                (y, 0, n_visible_lines - 1, Some(&mb.b))
            };

            ed.refresh_screen_w_minibuffer(Some(MiniBufferState {
                cx: x + offset,
                cy: screen_rows + 1 + n_visible_lines,
                prompt_line: &mb.prompt,
                selected_line_idx,
                b,
                top,
                bottom,
            }));

            match ed.block_for_key() {
                Key::Char(c) => {
                    input.insert(x, c);
                    x += 1;
                    mb.handle_on_change(&input, &on_change);
                }
                Key::Ctrl('h') | Key::Backspace | Key::Del => {
                    if x > 0 && x <= input.len() {
                        input.remove(x - 1);
                        x = x.saturating_sub(1);
                        mb.handle_on_change(&input, &on_change);
                    }
                }

                Key::Esc => return MiniBufferSelection::Cancelled,
                Key::Return => {
                    return match mb.b.line(y) {
                        Some(_) if line_indices.is_empty() => {
                            MiniBufferSelection::UserInput { input }
                        }
                        Some(l) => MiniBufferSelection::Line {
                            cy: line_indices[y],
                            line: l.to_string(),
                            input,
                        },
                        None => MiniBufferSelection::UserInput { input },
                    };
                }

                Key::Arrow(Arrow::Right) => x = min(x + 1, input.len()),
                Key::Arrow(Arrow::Left) => x = x.saturating_sub(1),
                Key::Alt('k') | Key::Arrow(Arrow::Up) => {
                    if selected_line_idx == 0 {
                        mb.b.set_dot(TextObject::BufferEnd, 1);
                    } else {
                        mb.b.set_dot(TextObject::Arr(Arrow::Up), 1);
                    }
                }
                Key::Alt('j') | Key::Arrow(Arrow::Down) => {
                    if selected_line_idx == visible_lines.len() - 1 {
                        mb.b.set_dot(TextObject::BufferStart, 1);
                    } else {
                        mb.b.set_dot(TextObject::Arr(Arrow::Down), 1);
                    }
                }

                _ => (),
            }
        }
    }

    pub fn prompt(prompt: &str, ed: &mut Editor) -> Option<String> {
        trace!(%prompt, "opening mini-buffer");
        match MiniBuffer::prompt_w_callback(prompt, vec![], |_| None, ed) {
            MiniBufferSelection::UserInput { input } => Some(input),
            _ => None,
        }
    }

    /// Append ", continue? [y/n]: " to the prompt and return true if the user enters one of
    /// y, Y, yes, YES, Yes (otherwise return false)
    pub fn confirm(prompt: &str, ed: &mut Editor) -> bool {
        let resp = MiniBuffer::prompt(&format!("{prompt}, continue? [y/n]: "), ed);

        matches!(resp.as_deref(), Some("y" | "Y" | "yes"))
    }

    pub fn select_from(
        prompt: &str,
        initial_lines: Vec<String>,
        ed: &mut Editor,
    ) -> MiniBufferSelection {
        MiniBuffer::prompt_w_callback(prompt, initial_lines, |_| None, ed)
    }

    pub fn select_from_command_output<S, I>(
        prompt: &str,
        cmd: &str,
        args: I,
        dir: &Path,
        ed: &mut Editor,
    ) -> MiniBufferSelection
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let initial_lines = match run_command(cmd, args, dir, ed.default_command_env()) {
            Ok(s) => s.lines().map(String::from).collect(),
            Err(e) => {
                ed.set_status_message(&format!("unable to get minibuffer input: {e}"));
                return MiniBufferSelection::Cancelled;
            }
        };

        MiniBuffer::prompt_w_callback(prompt, initial_lines, |_| None, ed)
    }
}
