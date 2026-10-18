//! Argument validation for the hand-parsed management commands.
//!
//! The agent tools (`fetch`, `search`, `crawl`, `screenshot`) are
//! clap-parsed, so an unknown or misspelled flag is refused with a
//! "similar argument" tip. The management commands parsed their own
//! argv and silently ignored whatever they did not recognize:
//! `donsetch doctor --frobnicate` ran the whole health check,
//! `donsetch status --json` printed the human view, `donsetch proxy
//! list --json` printed the table. A flag that does nothing is worse
//! than a flag that fails: the caller believes it asked for something.
//!
//! This module gives those commands the contract the tools already
//! have. One [`Spec`] per command lists what it accepts; [`Spec::reject`]
//! names the first unknown flag, suggests the nearest known one, prints
//! the usage line and exits 1, in the same shape clap uses.

use crate::cli;

/// The flags one command accepts.
pub struct Spec<'a> {
    /// Command name for the message (`doctor`, `proxy add`).
    pub cmd: &'a str,
    /// Usage line, printed on refusal and reused by the command's help.
    pub usage: &'a str,
    /// Flags that take no value.
    pub flags: &'a [&'a str],
    /// Flags that consume the next argument (`--status DOMAIN`).
    pub valued: &'a [&'a str],
}

impl<'a> Spec<'a> {
    pub const fn new(cmd: &'a str, usage: &'a str, flags: &'a [&'a str]) -> Self {
        Self {
            cmd,
            usage,
            flags,
            valued: &[],
        }
    }

    /// Add the value-taking flags (`--host HOST`).
    pub const fn with_valued(mut self, valued: &'a [&'a str]) -> Self {
        self.valued = valued;
        self
    }

    /// Every token this command understands, for validation and
    /// suggestions. `-h`/`--help` are always accepted: the routes that
    /// print help run before validation, but a nested command can still
    /// see them.
    fn allowed(&self) -> Vec<&'a str> {
        let mut v: Vec<&'a str> = Vec::with_capacity(self.flags.len() + self.valued.len() + 2);
        v.extend_from_slice(self.flags);
        v.extend_from_slice(self.valued);
        v.push("-h");
        v.push("--help");
        v
    }

    /// The first token that is neither a known flag nor a positional
    /// value. `--flag=value` is judged on its key; a lone `-` (stdin)
    /// and anything not starting with `-` are positional.
    pub fn first_unknown(&self, raw: &[String]) -> Option<String> {
        let allowed = self.allowed();
        raw.iter()
            .filter(|t| t.starts_with('-') && t.as_str() != "-")
            .map(|t| t.split('=').next().unwrap_or(t.as_str()).to_string())
            .find(|key| !allowed.contains(&key.as_str()))
    }

    /// Refuse an unknown flag: name it, suggest the nearest known one,
    /// print the usage, exit 1. Does nothing when every flag is known.
    pub fn reject(&self, raw: &[String]) {
        let Some(bad) = self.first_unknown(raw) else {
            return;
        };
        crate::diag!("error: unexpected argument '{bad}' found");
        if let Some(tip) = suggest(&bad, &self.allowed()) {
            crate::diag!("\n  tip: a similar argument exists: '{tip}'");
        }
        crate::diag!(
            "\nUsage: {}\n\nFor more information, try '--help'.",
            self.usage
        );
        std::process::exit(1);
    }
}

/// `donsetch <cmd>` never matches a command: say so, suggest the nearest
/// one, exit 1. `known` is the top-level command list.
pub fn unknown_command(cmd: &str, known: &[&str]) -> ! {
    crate::diag!("donsetch: unknown command '{cmd}'");
    if let Some(tip) = suggest(cmd, known) {
        crate::diag!("  tip: a similar command exists: '{tip}'");
    }
    crate::diag!("");
    cli::tool::print_top_help();
    std::process::exit(1);
}

/// The nearest candidate within an edit-distance budget that scales with
/// the typo, so `--depp` finds `--deep` and `--frobnicate` matches
/// nothing. `-h`/`--help` are never suggested: a misspelled flag is not
/// a request for help.
pub fn suggest<'a>(typo: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let budget = (typo.len() / 3).max(2);
    let mut best: Option<(&'a str, usize)> = None;
    for cand in candidates {
        if *cand == "-h" || *cand == "--help" {
            continue;
        }
        let d = distance(typo, cand);
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((cand, d));
        }
    }
    best.filter(|(_, d)| *d <= budget).map(|(c, _)| c)
}

/// Levenshtein distance, two-row. Inputs are flags, so they are short.
fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_misspelled_flag_finds_its_neighbour() {
        assert_eq!(suggest("--depp", &["--deep", "--fix"]), Some("--deep"));
        assert_eq!(suggest("--jsonn", &["--json", "--fix"]), Some("--json"));
        assert_eq!(suggest("--bogus", &["--focus"]), Some("--focus"));
    }

    #[test]
    fn a_nonsense_flag_suggests_nothing() {
        assert_eq!(
            suggest("--frobnicate", &["--deep", "--fix", "--json"]),
            None
        );
    }

    #[test]
    fn help_is_never_the_suggestion() {
        assert_eq!(
            suggest("--hlep", &["-h", "--help", "--deep"]),
            Some("--deep")
        );
    }

    #[test]
    fn known_flags_positionals_and_equals_forms_pass() {
        let spec = Spec::new("proxy add", "donsetch proxy add <url>", &["--no-check"])
            .with_valued(&["--timeout"]);
        let ok: Vec<String> = ["add", "http://1.2.3.4:8080", "--no-check", "--timeout=5"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(spec.first_unknown(&ok), None);
    }

    #[test]
    fn an_unknown_flag_is_named_by_key_not_by_value() {
        let spec = Spec::new("doctor", "donsetch doctor [--deep]", &["--deep"]);
        let raw: Vec<String> = ["--frobnicate=7"].iter().map(|s| s.to_string()).collect();
        assert_eq!(spec.first_unknown(&raw).as_deref(), Some("--frobnicate"));
    }

    #[test]
    fn a_lone_dash_is_stdin_not_a_flag() {
        let spec = Spec::new("keys export", "donsetch keys export [path|-]", &[]);
        let raw: Vec<String> = ["-"].iter().map(|s| s.to_string()).collect();
        assert_eq!(spec.first_unknown(&raw), None);
    }
}
