//! A minimal config file format for ad
use crate::term::Color;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub(crate) tabstop: usize,
    pub(crate) expand_tab: bool,
    pub(crate) match_indent: bool,
    pub(crate) status_timeout: u64,
    pub(crate) minibuffer_lines: usize,
    pub(crate) colorscheme: ColorScheme,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tabstop: 4,
            expand_tab: true,
            match_indent: true,
            status_timeout: 5,
            minibuffer_lines: 10,
            colorscheme: ColorScheme::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorScheme {
    pub(crate) bg: Color,
    pub(crate) fg: Color,
    pub(crate) dot_bg: Color,
    pub(crate) bar_bg: Color,
    pub(crate) signcol_fg: Color,
    pub(crate) minibuffer_hl: Color,
}

impl Default for ColorScheme {
    fn default() -> Self {
        Self {
            bg: "#1B1720".try_into().unwrap(),
            fg: "#EBDBB2".try_into().unwrap(),
            dot_bg: "#336677".try_into().unwrap(),
            bar_bg: "#4E415C".try_into().unwrap(),
            signcol_fg: "#544863".try_into().unwrap(),
            minibuffer_hl: "#3E3549".try_into().unwrap(),
        }
    }
}

impl Config {
    /// Attempt to parse the given file content as a Config file. If the file is invalid then an
    /// error message for the user is returned for displaying in the status bar.
    pub fn parse(contents: &str) -> Result<Self, String> {
        let mut cfg = Config::default();

        for line in contents.lines() {
            let line = line.trim_end();

            if line.starts_with('#') || line.is_empty() {
                continue;
            }

            match line.strip_prefix("set ") {
                None => return Err(format!("'{line}' is not a 'set prop=val' command")),
                Some(line) => cfg.try_set_prop(line)?,
            }
        }

        Ok(cfg)
    }

    pub(crate) fn try_set_prop(&mut self, input: &str) -> Result<(), String> {
        let (prop, val) = match input.split_once('=') {
            None => return Err(format!("'{input}' is not a 'set prop=val' command")),
            Some(parts) => parts,
        };

        match prop {
            // Numbers
            "tabstop" => self.tabstop = parse_usize(prop, val)?,
            "minibuffer-lines" => self.minibuffer_lines = parse_usize(prop, val)?,
            "status-timeout" => self.status_timeout = parse_usize(prop, val)? as u64,

            // Flags
            "expand-tab" => self.expand_tab = parse_bool(prop, val)?,
            "match-indent" => self.match_indent = parse_bool(prop, val)?,

            // Colors
            "bg-color" => self.colorscheme.bg = parse_color(prop, val)?,
            "fg-color" => self.colorscheme.fg = parse_color(prop, val)?,
            "dot-bg-color" => self.colorscheme.dot_bg = parse_color(prop, val)?,
            "bar-bg-color" => self.colorscheme.bar_bg = parse_color(prop, val)?,
            "signcol-fg-color" => self.colorscheme.signcol_fg = parse_color(prop, val)?,
            "minibuffer-hl-color" => self.colorscheme.minibuffer_hl = parse_color(prop, val)?,

            _ => return Err(format!("'{prop}' is not a known config property")),
        }

        Ok(())
    }
}

fn parse_usize(prop: &str, val: &str) -> Result<usize, String> {
    match val.parse() {
        Ok(num) => Ok(num),
        Err(_) => Err(format!("expected number for '{prop}' but found '{val}'")),
    }
}

fn parse_bool(prop: &str, val: &str) -> Result<bool, String> {
    match val {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!(
            "expected true/false for '{prop}' but found '{val}'"
        )),
    }
}

fn parse_color(prop: &str, val: &str) -> Result<Color, String> {
    Color::try_from(val)
        .map_err(|_| format!("expected #RRGGBB string for '{prop}' but found '{val}'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE_CONFIG: &str = include_str!("../data/init.conf");
    const CUSTOM_CONFIG: &str = "
# This is a comment


# Blank lines should be skipped
set tabstop=7

set expand-tab=false
set match-indent=false
";

    // This should be our default so we are just verifying that we have not diverged from
    // what is in the repo.
    #[test]
    fn parse_of_example_config_works() {
        let cfg = Config::parse(EXAMPLE_CONFIG).unwrap();

        let expected = Config {
            tabstop: 4,
            expand_tab: true,
            match_indent: true,
            ..Default::default()
        };

        assert_eq!(cfg, expected);
    }

    #[test]
    fn custom_vals_work() {
        let cfg = Config::parse(CUSTOM_CONFIG).unwrap();

        let expected = Config {
            tabstop: 7,
            expand_tab: false,
            match_indent: false,
            ..Default::default()
        };

        assert_eq!(cfg, expected);
    }
}
