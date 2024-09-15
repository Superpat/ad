//! Editor actions in response to user input
use crate::{
    buffer::{BufferKind, Buffers, MiniBuffer, MiniBufferSelection},
    config,
    config::Config,
    die,
    dot::{Cur, Dot, TextObject},
    editor::Editor,
    exec::{Addr, Address, Program},
    fsys::BufId,
    key::Key,
    mode::Mode,
    replace_config, update_config,
    util::{
        pipe_through_command, read_clipboard, run_command, run_command_blocking, set_clipboard,
    },
};
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
};
use tracing::{debug, error, info, trace, warn};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Actions {
    Single(Action),
    Multi(Vec<Action>),
}

/// How the current viewport should be set in relation to dot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewPort {
    /// Dot at the bottom of the viewport
    Bottom,
    /// Dot in the center of the viewport
    Center,
    /// Dot at the top of the viewport
    Top,
}

/// Supported actions for interacting with the editor state
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Action {
    AppendToOutputBuffer { bufid: usize, content: String },
    ChangeDirectory { path: Option<String> },
    CommandMode,
    Delete,
    DeleteBuffer { force: bool },
    DotCollapseFirst,
    DotCollapseLast,
    DotExtendBackward(TextObject, usize),
    DotExtendForward(TextObject, usize),
    DotFlip,
    DotSet(TextObject, usize),
    EditCommand { cmd: String },
    ExecuteDot,
    Exit { force: bool },
    FindFile,
    FindRepoFile,
    FocusBuffer { id: usize },
    InsertChar { c: char },
    InsertString { s: String },
    JumpListForward,
    JumpListBack,
    LoadDot,
    NewEditLogTransaction,
    NextBuffer,
    OpenFile { path: String },
    Paste,
    PreviousBuffer,
    RawKey { k: Key },
    Redo,
    ReloadActiveBuffer,
    ReloadBuffer { id: usize },
    ReloadConfig,
    RunMode,
    SamMode,
    SaveBuffer { force: bool },
    SaveBufferAs { path: String, force: bool },
    SearchInCurrentBuffer,
    SelectBuffer,
    SetConfigProp { input: String },
    SetViewPort(ViewPort),
    SetMode { m: &'static str },
    SetStatusMessage { message: String },
    ShellPipe { cmd: String },
    ShellReplace { cmd: String },
    ShellRun { cmd: String },
    ShellSend { cmd: String },
    Undo,
    ViewLogs,
    Yank,

    DebugBufferContents,
    DebugEditLog,
}

impl Editor {
    pub(crate) fn change_directory(&mut self, opt_path: Option<String>) {
        let p = match opt_path {
            Some(p) => p,
            None => match env::var("HOME") {
                Ok(p) => p,
                Err(e) => {
                    let msg = format!("Unable to determine home directory: {e}");
                    self.set_status_message(&msg);
                    warn!("{msg}");
                    return;
                }
            },
        };

        let new_cwd = match fs::canonicalize(p) {
            Ok(cwd) => cwd,
            Err(e) => {
                self.set_status_message(&format!("Invalid path: {e}"));
                return;
            }
        };

        if let Err(e) = env::set_current_dir(&new_cwd) {
            let msg = format!("Unable to set working directory: {e}");
            self.set_status_message(&msg);
            error!("{msg}");
            return;
        };

        debug!(new_cwd=%new_cwd.as_os_str().to_string_lossy(), "setting working directory");
        self.cwd = new_cwd;
        self.set_status_message(&self.cwd.display().to_string());
    }

    /// Open a file within the editor using a path that is relative to the current working
    /// directory
    pub fn open_file_relative_to_cwd(&mut self, path: &str) {
        self.open_file(self.cwd.join(path));
    }

    /// Open a file within the editor
    pub fn open_file<P: AsRef<Path>>(&mut self, path: P) {
        let path = path.as_ref();
        debug!(?path, "opening file");
        let was_empty_scratch = self.buffers.is_empty_scratch();
        let current_id = self.buffers.active().id;

        match self.buffers.open_or_focus(path) {
            Err(e) => self.set_status_message(&format!("Error opening file: {e}")),

            Ok(Some(new_id)) => {
                if was_empty_scratch {
                    self.tx_fsys.send(BufId::Remove(current_id)).unwrap();
                }
                self.tx_fsys.send(BufId::Add(new_id)).unwrap();
                self.tx_fsys.send(BufId::Current(new_id)).unwrap();
            }

            Ok(None) => {
                match self.buffers.active().state_changed_on_disk() {
                    Ok(true) => {
                        let res = MiniBuffer::prompt("File changed on disk, reload? [y/n]: ", self);
                        if let Some("y" | "Y" | "yes") = res.as_deref() {
                            let msg = self.buffers.active_mut().reload_from_disk();
                            self.set_status_message(&msg);
                        }
                    }
                    Ok(false) => (),
                    Err(e) => self.set_status_message(&e),
                }
                let id = self.buffers.active().id;
                self.tx_fsys.send(BufId::Current(id)).unwrap();
            }
        };
    }

    fn find_file_under_dir(&mut self, d: &Path) {
        let cmd = config!().find_command.clone();

        let selection = match cmd.split_once(' ') {
            Some((cmd, args)) => {
                MiniBuffer::select_from_command_output("> ", cmd, args.split_whitespace(), d, self)
            }
            None => MiniBuffer::select_from_command_output(
                "> ",
                &cmd,
                std::iter::empty::<&str>(),
                d,
                self,
            ),
        };

        if let MiniBufferSelection::Line { line, .. } = selection {
            self.open_file_relative_to_cwd(&format!("{}/{}", d.display(), line.trim()));
        }
    }

    /// This shells out to the fd command line program
    pub(crate) fn find_file(&mut self) {
        let d = self.buffers.active().dir().unwrap_or(&self.cwd).to_owned();
        self.find_file_under_dir(&d);
    }

    /// This shells out to the git and fd command line programs
    pub(crate) fn find_repo_file(&mut self) {
        let d = self.buffers.active().dir().unwrap_or(&self.cwd).to_owned();
        let s = match run_command_blocking(
            "git",
            ["rev-parse", "--show-toplevel"],
            &d,
            self.active_buffer_id(),
        ) {
            Ok(s) => s,
            Err(e) => {
                self.set_status_message(&format!("unable to find git root: {e}"));
                return;
            }
        };

        let root = Path::new(s.trim());
        self.find_file_under_dir(root);
    }

    pub(crate) fn delete_buffer(&mut self, id: usize, force: bool) {
        match self.buffers.with_id(id) {
            Some(b) if b.dirty && !force => self.set_status_message("No write since last change"),
            None => warn!("attempt to close unknown buffer, id={id}"),
            _ => {
                let is_last_buffer = self.buffers.len() == 1;
                self.tx_fsys.send(BufId::Remove(id)).unwrap();
                self.buffers.close_buffer(id);
                self.running = !is_last_buffer;
            }
        }
    }

    pub(super) fn save_current_buffer(&mut self, fname: Option<String>, force: bool) {
        trace!("attempting to save current buffer");
        let p = match self.get_buffer_save_path(fname) {
            Some(p) => p,
            None => return,
        };

        let msg = self.buffers.active_mut().save_to_disk_at(p, force);
        self.set_status_message(&msg);
    }

    fn get_buffer_save_path(&mut self, fname: Option<String>) -> Option<PathBuf> {
        use BufferKind as Bk;

        let desired_path = match (fname, &self.buffers.active().kind) {
            // File has a known name which is either where we loaded it from or a
            // path that has been set and verified from the Some(s) case that follows
            (None, Bk::File(ref p)) => return Some(p.clone()),
            // Renaming an existing file or attempting to save a new file created in
            // the editor: both need verifying
            (Some(s), Bk::File(_) | Bk::Unnamed) => PathBuf::from(s),
            // Attempting to save without a name so we prompt for one and verify it
            (None, Bk::Unnamed) => match MiniBuffer::prompt("Save As: ", self) {
                Some(s) => s.into(),
                None => return None,
            },
            // virtual and minibuffer buffers don't support saving and have no save path
            (_, Bk::Virtual(_) | Bk::Output(_) | Bk::MiniBuffer) => return None,
        };

        match desired_path.try_exists() {
            Ok(false) => (),
            Ok(true) => {
                if !MiniBuffer::confirm("File already exists", self) {
                    return None;
                }
            }
            Err(e) => {
                self.set_status_message(&format!("Unable to check path: {e}"));
                return None;
            }
        }

        self.buffers.active_mut().kind = BufferKind::File(desired_path.clone());

        Some(desired_path)
    }

    pub(super) fn reload_buffer(&mut self, id: usize) {
        let msg = match self.buffers.with_id_mut(id) {
            Some(b) => b.reload_from_disk(),
            // Silently ignoring attempts to reload unknown buffers
            None => return,
        };

        self.set_status_message(&msg);
    }

    pub(super) fn reload_config(&mut self) {
        info!("reloading config");
        let msg = match Config::try_load() {
            Ok(config) => {
                replace_config(config);
                "config reloaded".to_string()
            }
            Err(s) => s,
        };
        info!("{msg}");

        self.set_status_message(&msg);
    }

    pub(super) fn reload_active_buffer(&mut self) {
        let msg = self.buffers.active_mut().reload_from_disk();
        self.set_status_message(&msg);
    }

    pub(super) fn set_config_prop(&mut self, input: &str) {
        info!(%input, "setting config property");
        if let Err(msg) = update_config(input) {
            self.set_status_message(&msg);
        }
    }

    pub(super) fn set_mode(&mut self, name: &str) {
        if let Some((i, _)) = self.modes.iter().enumerate().find(|(_, m)| m.name == name) {
            self.modes.swap(0, i);
            let cur_shape = self.modes[0].cur_shape.to_string();
            if let Err(e) = self.stdout.write_all(cur_shape.as_bytes()) {
                // In this situation we're probably not going to be able to do all that much
                // but we might as well try
                die!("Unable to write to stdout: {e}");
            };
        }
    }

    pub(super) fn exit(&mut self, force: bool) {
        let dirty_buffers = self.buffers.dirty_buffers();
        if !dirty_buffers.is_empty() && !force {
            self.set_status_message("No write since last change. Use ':q!' to force exit");
            MiniBuffer::select_from("No write since last change> ", dirty_buffers, self);
            return;
        }

        self.running = false;
    }

    pub(super) fn set_clipboard(&mut self, s: String) {
        trace!("setting clipboard content");
        match set_clipboard(&s) {
            Ok(_) => self.set_status_message("Yanked selection to system clipboard"),
            Err(e) => self.set_status_message(&format!("Error setting system clipboard: {e}")),
        }
    }

    pub(super) fn paste_from_clipboard(&mut self) {
        trace!("pasting from clipboard");
        match read_clipboard() {
            Ok(s) => self.handle_action(Action::InsertString { s }),
            Err(e) => self.set_status_message(&format!("Error reading system clipboard: {e}")),
        }
    }

    pub(super) fn search_in_current_buffer(&mut self) {
        let numbered_lines = self
            .buffers
            .active()
            .string_lines()
            .into_iter()
            .enumerate()
            .map(|(i, line)| format!("{:>4} | {}", i + 1, line))
            .collect();

        let selection = MiniBuffer::select_from("> ", numbered_lines, self);
        if let MiniBufferSelection::Line { cy, .. } = selection {
            self.buffers.active_mut().dot = Dot::Cur {
                c: Cur::from_yx(cy, 0, self.buffers.active()),
            };
            self.handle_action(Action::DotSet(TextObject::Line, 1));
            self.handle_action(Action::SetViewPort(ViewPort::Center));
        }
    }

    pub(super) fn select_buffer(&mut self) {
        let selection = MiniBuffer::select_from("> ", self.buffers.as_buf_list(), self);
        if let MiniBufferSelection::Line { line, .. } = selection {
            // unwrap is fine here because we know the format of the buf list we are supplying
            if let Ok(id) = line.split_once(' ').unwrap().0.parse::<usize>() {
                self.focus_buffer(id);
            }
        }
    }

    pub(super) fn focus_buffer(&mut self, id: usize) {
        self.buffers.focus_id(id);
        self.tx_fsys.send(BufId::Current(id)).unwrap();
    }

    pub(super) fn debug_buffer_contents(&mut self) {
        MiniBuffer::select_from(
            "<RAW BUFFER> ",
            self.buffers
                .active()
                .string_lines()
                .into_iter()
                .map(|l| format!("{:?}", l))
                .collect(),
            self,
        );
    }

    pub(super) fn view_logs(&mut self) {
        self.open_virtual("+logs", self.log_buffer.content())
    }

    pub(super) fn debug_edit_log(&mut self) {
        MiniBuffer::select_from("<EDIT LOG> ", self.buffers.active().debug_edit_log(), self);
    }

    // TODO: implement customisation of load and execute via the events file once that is in place.

    /// Default semantics for attempting to load the current dot:
    ///   - an absolute path -> open in ad
    ///   - a relative path from the directory of the containing file -> open in ad
    ///     - if either have a valid addr following a colon then set dot to that addr
    ///   - search within the current buffer for the next occurance of dot and select it
    ///
    /// Loading and executing of dot is part of what makes ad an unsual editor. The semantics are
    /// lifted almost directly from acme on plan9 and the curious user is encouraged to read the
    /// materials available at http://acme.cat-v.org/ to learn more about what is possible with
    /// such a system.
    pub(super) fn default_load_dot(&mut self) {
        let b = self.buffers.active_mut();
        b.expand_cur_dot();

        let dot = b.dot.content(b);

        let (maybe_path, maybe_addr) = match dot.find(':') {
            Some(idx) => {
                let (s, addr) = dot.split_at(idx);
                let (_, addr) = addr.split_at(1);
                match Addr::parse(&mut addr.chars().peekable()) {
                    Ok(expr) => (s, Some(expr)),
                    Err(_) => (s, None),
                }
            }
            None => (dot.as_str(), None),
        };

        let try_set_addr = |buffers: &mut Buffers| {
            if let Some(mut addr) = maybe_addr {
                let b = buffers.active_mut();
                b.dot = b.map_addr(&mut addr);
            }
        };

        let path = Path::new(&maybe_path);

        if path.exists() {
            self.open_file(path);
            return try_set_addr(&mut self.buffers);
        }

        if let Some(parent) = b.dir() {
            let full_path = parent.join(path);
            if full_path.exists() {
                self.open_file(full_path);
                return try_set_addr(&mut self.buffers);
            }
        }

        b.find_forward(&dot);
    }

    /// Default semantics for attempting to execute the current dot:
    ///   - a valid ad command -> execute the command
    ///   - attempt to run as a shell command with args
    ///
    /// Loading and executing of dot is part of what makes ad an unsual editor. The semantics are
    /// lifted almost directly from acme on plan9 and the curious user is encouraged to read the
    /// materials available at http://acme.cat-v.org/ to learn more about what is possible with
    /// such a system.
    pub(super) fn default_execute_dot(&mut self) {
        let b = self.buffers.active_mut();
        b.expand_cur_dot();
        let cmd = b.dot.content(b);

        match self.parse_command(cmd.trim_end()) {
            Some(actions) => self.handle_actions(actions),
            None => self.run_shell_cmd(&cmd),
        }
    }

    pub(super) fn execute_command(&mut self, cmd: &str) {
        debug!(%cmd, "executing command");
        if let Some(actions) = self.parse_command(cmd.trim_end()) {
            self.handle_actions(actions);
        }
    }

    pub(super) fn execute_edit_command(&mut self, cmd: &str) {
        debug!(%cmd, "executing edit command");
        let mut prog = match Program::try_parse(cmd) {
            Ok(prog) => prog,
            Err(error) => {
                warn!(?error, "invalid edit command");
                self.set_status_message(&format!("Invalid edit command: {error:?}"));
                return;
            }
        };

        let mut buf = Vec::new();
        let fname = self.buffers.active().full_name().to_string();
        match prog.execute(self.buffers.active_mut(), &fname, &mut buf) {
            Ok(new_dot) => self.buffers.active_mut().dot = new_dot,
            Err(e) => self.set_status_message(&format!("Error running edit command: {e:?}")),
        }

        // FIXME: this is just using a selection mini-buffer for now to test things out. Ideally
        // this should be a scratchpad that we can dismiss and bring back but that will require
        // support in the main Buffers struct and a new way of creating a MiniBuffer.
        if !buf.is_empty() {
            MiniBuffer::select_from(
                "%>",
                String::from_utf8(buf)
                    .unwrap()
                    .lines()
                    .map(|l| l.to_string())
                    .collect(),
                self,
            );
        }
    }

    pub(super) fn command_mode(&mut self) {
        self.modes.insert(0, Mode::ephemeral_mode("COMMAND"));

        if let Some(input) = MiniBuffer::prompt(":", self) {
            self.execute_command(&input);
        }

        self.modes.remove(0);
    }

    pub(super) fn run_mode(&mut self) {
        self.modes.insert(0, Mode::ephemeral_mode("RUN"));

        if let Some(input) = MiniBuffer::prompt("!", self) {
            self.run_shell_cmd(&input);
        }

        self.modes.remove(0);
    }

    pub(super) fn sam_mode(&mut self) {
        self.modes.insert(0, Mode::ephemeral_mode("EDIT"));

        if let Some(input) = MiniBuffer::prompt("Edit> ", self) {
            self.execute_edit_command(&input);
        };

        self.modes.remove(0);
    }

    pub(super) fn pipe_dot_through_shell_cmd(&mut self, raw_cmd_str: &str) {
        let (s, d) = {
            let b = self.buffers.active();
            (b.dot_contents(), b.dir().unwrap_or(&self.cwd))
        };

        let id = self.active_buffer_id();
        let res = match raw_cmd_str.split_once(' ') {
            Some((cmd, rest)) => pipe_through_command(cmd, rest.split_whitespace(), &s, d, id),
            None => pipe_through_command(raw_cmd_str, std::iter::empty::<&str>(), &s, d, id),
        };

        match res {
            Ok(s) => self.handle_action(Action::InsertString { s }),
            Err(e) => self.set_status_message(&format!("Error running external command: {e}")),
        }
    }

    pub(super) fn replace_dot_with_shell_cmd(&mut self, raw_cmd_str: &str) {
        let d = self.buffers.active().dir().unwrap_or(&self.cwd);
        let id = self.active_buffer_id();
        let res = match raw_cmd_str.split_once(' ') {
            Some((cmd, rest)) => run_command_blocking(cmd, rest.split_whitespace(), d, id),
            None => run_command_blocking(raw_cmd_str, std::iter::empty::<&str>(), d, id),
        };

        match res {
            Ok(s) => self.handle_action(Action::InsertString { s }),
            Err(e) => self.set_status_message(&format!("Error running external command: {e}")),
        }
    }

    pub(super) fn run_shell_cmd(&mut self, raw_cmd_str: &str) {
        let d = self.buffers.active().dir().unwrap_or(&self.cwd);
        let id = self.active_buffer_id();
        match raw_cmd_str.split_once(' ') {
            Some((cmd, rest)) => run_command(
                cmd,
                rest.split_whitespace(),
                d,
                id,
                self.tx_input_events.clone(),
            ),
            None => run_command(
                raw_cmd_str,
                std::iter::empty::<&str>(),
                d,
                id,
                self.tx_input_events.clone(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{editor::EditorMode, LogBuffer};
    use simple_test_case::test_case;

    macro_rules! assert_recv {
        ($brx:expr, $msg:ident, $expected:expr) => {
            match $brx.try_recv() {
                Ok(BufId::$msg(id)) if id == $expected => (),
                Ok(msg) => panic!(
                    "expected {}({}) but got {msg:?}",
                    stringify!($msg),
                    $expected
                ),
                Err(_) => panic!("recv {}({})", stringify!($msg), $expected),
            }
        };
    }

    #[test]
    fn opening_a_file_sends_the_correct_fsys_messages() {
        let mut ed = Editor::new(
            Config::default(),
            EditorMode::Headless,
            LogBuffer::default(),
        );
        let brx = ed.rx_fsys.take().expect("to have fsys channels");

        ed.open_file("foo");

        // The first open should also close our scratch buffer
        assert_recv!(brx, Remove, 0);
        assert_recv!(brx, Add, 1);
        assert_recv!(brx, Current, 1);

        // Opening a second file should only notify for that file
        ed.open_file("bar");
        assert_recv!(brx, Add, 2);
        assert_recv!(brx, Current, 2);

        // Opening the first file again should just notify for the current file
        ed.open_file("foo");
        assert_recv!(brx, Current, 1);
    }

    #[test_case(&[], &[0]; "empty scratch")]
    #[test_case(&["foo"], &[1]; "one file")]
    #[test_case(&["foo", "bar"], &[1, 2]; "two files")]
    #[test]
    fn ensure_correct_fsys_state_works(files: &[&str], expected_ids: &[usize]) {
        let mut ed = Editor::new(
            Config::default(),
            EditorMode::Headless,
            LogBuffer::default(),
        );
        let brx = ed.rx_fsys.take().expect("to have fsys channels");

        for file in files {
            ed.open_file(file);
        }

        ed.ensure_correct_fsys_state();

        if !files.is_empty() {
            assert_recv!(brx, Remove, 0);
        }

        for &expected in expected_ids {
            assert_recv!(brx, Add, expected);
            assert_recv!(brx, Current, expected);
        }
    }
}
