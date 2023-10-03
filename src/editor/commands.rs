//! Command mode commands for ad
use crate::editor::{
    Action::*,
    Actions::{self, *},
    Editor,
};

impl Editor {
    pub(super) fn parse_command(&mut self, input: &str) -> Option<Actions> {
        if let Some(actions) = try_parse_single_char_command(input) {
            return Some(actions);
        }

        let (command, args) = if input.contains(' ') {
            input.split_once(' ')?
        } else {
            (input, "")
        };

        match command {
            "b" | "buffer" => Some(Single(SelectBuffer)),
            "bn" | "buffer-next" => Some(Single(NextBuffer)),
            "bp" | "buffer-previous" => Some(Single(PreviousBuffer)),

            "cd" | "change-directory" => {
                if args.is_empty() {
                    Some(Single(ChangeDirectory { path: None }))
                } else {
                    Some(Single(ChangeDirectory {
                        path: Some(args.to_string()),
                    }))
                }
            }

            "db" | "delete-buffer" => Some(Single(DeleteBuffer { force: false })),
            "db!" | "delete-buffer!" => Some(Single(DeleteBuffer { force: true })),

            "e" | "edit" => {
                if args.is_empty() {
                    self.set_status_message("No filename provided");
                    None
                } else {
                    Some(Single(OpenFile {
                        path: args.to_string(),
                    }))
                }
            }

            "q" | "quit" => Some(Single(Exit { force: false })),
            "q!" | "quit!" => Some(Single(Exit { force: true })),

            "w" | "write" => {
                if args.is_empty() {
                    Some(Single(SaveBuffer))
                } else {
                    Some(Single(SaveBufferAs {
                        path: args.to_string(),
                    }))
                }
            }

            "wq" | "write-quit" => Some(Multi(vec![SaveBuffer, Exit { force: false }])),
            "wq!" | "write-quit!" => Some(Multi(vec![SaveBuffer, Exit { force: true }])),

            "" => None,

            _ => {
                self.set_status_message(&format!("Not an editor command: {command}"));
                None
            }
        }
    }
}

fn try_parse_single_char_command(input: &str) -> Option<Actions> {
    return match input.chars().next() {
        Some('!') => Some(Single(ShellRun {
            cmd: input[1..].to_string(),
        })),
        Some('|') => Some(Single(ShellPipe {
            cmd: input[1..].to_string(),
        })),
        Some('<') => Some(Single(ShellReplace {
            cmd: input[1..].to_string(),
        })),
        Some('>') => Some(Single(ShellSend {
            cmd: input[1..].to_string(),
        })),

        _ => None,
    };
}
