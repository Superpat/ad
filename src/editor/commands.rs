//! Command mode commands for ad
use crate::editor::{
    Action::*,
    Actions::{self, *},
    Editor,
};

impl Editor {
    pub(super) fn parse_command(&mut self, input: &str) -> Option<Actions> {
        let (command, args) = if input.contains(' ') {
            input.split_once(' ')?
        } else {
            (input, "")
        };

        match command {
            "bc" | "buffer-close" => Some(Single(CloseBuffer)),
            "bn" | "buffer-next" => Some(Single(NextBuffer)),
            "bp" | "buffer-previous" => Some(Single(PreviousBuffer)),

            "e" | "edit" => {
                if args.is_empty() {
                    self.set_status_message("No filename provided");
                    None
                } else {
                    Some(Multi(vec![OpenFile {
                        path: args.to_string(),
                    }]))
                }
            }

            "q" | "quit" => Some(Single(Exit { force: false })),
            "Q" | "quit-all" => Some(Single(Exit { force: true })),

            "w" | "write" => {
                if args.is_empty() {
                    Some(Single(SaveBuffer))
                } else {
                    Some(Single(SaveBufferAs {
                        path: args.to_string(),
                    }))
                }
            }

            "" => None,

            _ => {
                self.set_status_message(&format!("Not an editor command: {input}"));
                None
            }
        }
    }
}
