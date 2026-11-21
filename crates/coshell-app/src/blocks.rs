//! Command blocks: the terminal's text split at shell prompts, so each command
//! and its output can be drawn as one card (who ran it, prompt, time, output).
//!
//! The PTY stays the source of truth: blocks are re-read from the screen and
//! scrollback, and keys still go to the shell.

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use coshell_term::SharedTerm;
use coshell_term::alacritty_terminal::term::TermMode;
use regex::Regex;

/// Rows of screen + scrollback turned into blocks.
pub const BLOCK_ROWS: usize = 1500;

/// `user@host:cwd$ cmd`, `user@host cwd % cmd`, `[user@host cwd]$ cmd`.
static PROMPT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\[?(?P<uh>[A-Za-z0-9._-]+@[A-Za-z0-9._-]+)(?::\s?|\s+)(?P<cwd>[^\s\]$#%>]*)\]?\s?(?P<sym>[$#%>])(?:\s(?P<cmd>.*))?$")
        .expect("prompt regex")
});

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub user_host: String,
    pub cwd: String,
    pub sym: String,
}

/// How prompt lines are recognised in this session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sig {
    /// `user@host` of a standard prompt.
    UserHost(String),
    /// Any other prompt: the exact text before the cursor.
    Raw(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub prompt: Prompt,
    pub cmd: String,
    pub output: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Screen {
    /// Text above the first prompt (login banner, MOTD).
    pub preamble: Vec<String>,
    pub blocks: Vec<Block>,
    /// The prompt waiting for input and what is typed on it; `None` while a command runs.
    pub current: Option<(Prompt, String)>,
}

impl Screen {
    pub fn running(&self) -> bool {
        self.current.is_none() && !self.blocks.is_empty()
    }
}

/// A standard prompt line split into its parts and the typed command.
pub fn parse_prompt(line: &str) -> Option<(Prompt, String)> {
    let c = PROMPT.captures(line.trim_end())?;
    let prompt = Prompt { user_host: c["uh"].to_string(), cwd: c["cwd"].to_string(), sym: c["sym"].to_string() };
    Some((prompt, c.name("cmd").map_or("", |m| m.as_str()).trim().to_string()))
}

/// The signature of a bare prompt at the bottom of the screen, if it is one.
pub fn sig_of(last: &str) -> Option<Sig> {
    match parse_prompt(last) {
        Some((p, cmd)) if cmd.is_empty() => Some(Sig::UserHost(p.user_host)),
        Some(_) => None,
        None if crate::assist::looks_like_prompt(last) => Some(Sig::Raw(last.trim_end().to_string())),
        None => None,
    }
}

fn prompt_line(line: &str, sig: &Sig) -> Option<(Prompt, String)> {
    match sig {
        Sig::UserHost(uh) => parse_prompt(line).filter(|(p, _)| p.user_host == *uh),
        Sig::Raw(raw) => line.strip_prefix(raw.as_str()).map(|rest| {
            (Prompt { user_host: raw.clone(), cwd: String::new(), sym: String::new() }, rest.trim().to_string())
        }),
    }
}

/// Split terminal lines into blocks at prompt lines.
pub fn parse(lines: &[String], sig: &Sig) -> Screen {
    let mut screen = Screen::default();
    for line in lines {
        match prompt_line(line, sig) {
            Some((prompt, cmd)) => screen.blocks.push(Block { prompt, cmd, output: Vec::new() }),
            None => match screen.blocks.last_mut() {
                Some(b) => b.output.push(line.clone()),
                None => screen.preamble.push(line.clone()),
            },
        }
    }
    // The last prompt with nothing after it is the one waiting for input.
    if let Some(b) = screen.blocks.pop_if(|b| b.output.is_empty()) {
        screen.current = Some((b.prompt, b.cmd));
    }
    // Enter on an empty line leaves a bare prompt behind.
    screen.blocks.retain(|b| !(b.cmd.is_empty() && b.output.iter().all(|l| l.trim().is_empty())));
    screen
}

/// Characters after the cursor on its row (the caret's distance from the end of the typed text).
pub fn cursor_tail(term: &SharedTerm) -> usize {
    use coshell_term::alacritty_terminal::grid::Dimensions;
    use coshell_term::alacritty_terminal::index::Column;
    let term = term.lock();
    let grid = term.grid();
    let cursor = grid.cursor.point;
    let row = &grid[cursor.line];
    let tail: String = (cursor.column.0..grid.columns()).map(|c| row[Column(c)].c).collect();
    tail.trim_end().chars().count()
}

/// Blank cells right before the cursor when nothing follows it on its row: spaces typed at
/// the end of the command. Lines read off the screen lose them (a trailing space and an
/// empty cell look the same), so only the cursor tells them apart.
pub fn cursor_gap(term: &SharedTerm) -> usize {
    use coshell_term::alacritty_terminal::grid::Dimensions;
    use coshell_term::alacritty_terminal::index::Column;
    let term = term.lock();
    let grid = term.grid();
    let cursor = grid.cursor.point;
    let row = &grid[cursor.line];
    let blank = |c: usize| row[Column(c)].c == ' ';
    if !(cursor.column.0..grid.columns()).all(blank) {
        return 0;
    }
    (0..cursor.column.0).rev().take_while(|&c| blank(c)).count()
}

/// A full-screen program (vim, top, less) owns the screen: show the raw terminal.
pub fn alt_screen(term: &SharedTerm) -> bool {
    term.lock().mode().contains(TermMode::ALT_SCREEN)
}

/// Who ran a block's command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    User,
    Ai,
}

/// What CoShell knows about a command it saw start: shown on the matching block.
#[derive(Clone, Debug)]
pub struct Meta {
    pub cmd: String,
    pub source: Source,
    /// Local wall-clock time it started (HH:MM:SS).
    pub time: String,
    pub started: Instant,
    pub took: Option<Duration>,
    pub exit: Option<u32>,
}

impl Meta {
    pub fn new(cmd: &str, source: Source) -> Self {
        Self { cmd: cmd.trim().to_string(), source, time: clock_now(), started: Instant::now(), took: None, exit: None }
    }
}

/// Metas for `blocks`, matched from the newest by command text.
pub fn match_metas<'a>(blocks: &[Block], metas: &'a [Meta]) -> Vec<Option<&'a Meta>> {
    let mut out = vec![None; blocks.len()];
    let mut m = metas.len();
    for (i, b) in blocks.iter().enumerate().rev() {
        if let Some(j) = metas[..m].iter().rposition(|x| x.cmd == b.cmd.trim()) {
            out[i] = Some(&metas[j]);
            m = j;
        }
    }
    out
}

/// Local time of day as HH:MM:SS.
pub fn clock_now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let t = (secs + local_offset_secs()).rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", t / 3600, t % 3600 / 60, t % 60)
}

/// The local UTC offset, read once from `date +%z` (no time zone database needed).
fn local_offset_secs() -> i64 {
    static OFFSET: LazyLock<i64> = LazyLock::new(|| {
        #[cfg(unix)]
        {
            let out = std::process::Command::new("date").arg("+%z").output().ok();
            let s = out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
            if s.len() == 5 {
                let sign = if s.starts_with('-') { -1 } else { 1 };
                let h: i64 = s[1..3].parse().unwrap_or(0);
                let m: i64 = s[3..5].parse().unwrap_or(0);
                return sign * (h * 3600 + m * 60);
            }
        }
        0
    });
    *OFFSET
}

/// Commands offered as inline completions besides the session's own history.
pub const KNOWN_COMMANDS: &[&str] = &[
    "df -h",
    "du -h -d 1 /var/log",
    "free -h",
    "uptime",
    "uname -a",
    "whoami",
    "ps aux --sort=-%cpu | head -n 6",
    "journalctl -p err -n 15 --no-pager",
    "systemctl status ",
    "docker ps",
    "ss -tlnp",
    "ls -la",
    "clear",
];

/// The rest of the newest history entry (then a known command) that starts with `typed`.
pub fn ghost(typed: &str, metas: &[Meta]) -> Option<String> {
    if typed.is_empty() || typed.trim_start().starts_with('#') {
        return None;
    }
    let lower = typed.to_lowercase();
    metas
        .iter()
        .rev()
        .map(|m| m.cmd.as_str())
        .chain(KNOWN_COMMANDS.iter().copied())
        .find(|c| c.to_lowercase().starts_with(&lower) && c.len() > typed.len())
        .map(|c| c[typed.len()..].to_string())
}

/// Kinds of highlighted output text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Plain,
    /// ≥ 85 % (disk, CPU …)
    High,
    Warn,
    Err,
    Ok,
    Path,
    /// Matches the search query.
    Found,
}

static MARKS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?P<high>\b(?:8[5-9]|9\d|100)%)|(?P<warn>\[WARN\]|\[warn\])|(?P<err>\[ERR\]|\[ERROR\]|\berror\b|\bfailed\b)|(?P<ok>\[\x{C644}\x{B8CC}\]|active \(running\)|active \(exited\))|(?P<path>/(?:var|etc|srv|home|dev|opt|usr|tmp)/\S+)",
    )
    .expect("marks regex")
});

fn marks_of(s: &str, out: &mut Vec<(String, Mark)>) {
    let mut at = 0;
    for c in MARKS.captures_iter(s) {
        let m = c.get(0).expect("whole match");
        if m.start() > at {
            out.push((s[at..m.start()].to_string(), Mark::Plain));
        }
        let kind = if c.name("high").is_some() {
            Mark::High
        } else if c.name("warn").is_some() {
            Mark::Warn
        } else if c.name("err").is_some() {
            Mark::Err
        } else if c.name("ok").is_some() {
            Mark::Ok
        } else {
            Mark::Path
        };
        out.push((m.as_str().to_string(), kind));
        at = m.end();
    }
    if at < s.len() {
        out.push((s[at..].to_string(), Mark::Plain));
    }
}

/// Output text cut into highlighted runs; `query` matches (any case) win.
pub fn highlight(text: &str, query: &str) -> Vec<(String, Mark)> {
    let mut out = Vec::new();
    let q = query.trim();
    if q.is_empty() {
        marks_of(text, &mut out);
        return out;
    }
    let re = Regex::new(&format!("(?i){}", regex::escape(q))).expect("escaped query");
    let mut at = 0;
    for m in re.find_iter(text) {
        marks_of(&text[at..m.start()], &mut out);
        out.push((m.as_str().to_string(), Mark::Found));
        at = m.end();
    }
    marks_of(&text[at..], &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(str::to_string).collect()
    }

    #[test]
    fn prompts_of_common_shells() {
        let (p, cmd) = parse_prompt("ops@app-01:~$ df -h /var").unwrap();
        assert_eq!((p.user_host.as_str(), p.cwd.as_str(), p.sym.as_str(), cmd.as_str()), ("ops@app-01", "~", "$", "df -h /var"));
        let (p, cmd) = parse_prompt("axiom@Mac ~ % ls").unwrap();
        assert_eq!((p.cwd.as_str(), p.sym.as_str(), cmd.as_str()), ("~", "%", "ls"));
        let (p, cmd) = parse_prompt("root@db:/var/log#").unwrap();
        assert_eq!((p.cwd.as_str(), p.sym.as_str(), cmd.as_str()), ("/var/log", "#", ""));
        let (p, _) = parse_prompt("[ops@web ~]$ uptime").unwrap();
        assert_eq!(p.user_host, "ops@web");
        assert!(parse_prompt("Filesystem Size Used").is_none());
        assert!(parse_prompt("# a comment in a file").is_none());
    }

    #[test]
    fn screen_splits_at_prompts_and_keeps_the_waiting_one() {
        let sig = Sig::UserHost("ops@app-01".into());
        let s = parse(
            &lines("Welcome to Ubuntu\nops@app-01:~$ df -h /var\nFilesystem Size\n/dev/sda2 80G 91% /var\nops@app-01:~$\nops@app-01:~$ du -h\n56G /var/log\nops@app-01:~$ ls -l"),
            &sig,
        );
        assert_eq!(s.preamble, vec!["Welcome to Ubuntu"]);
        assert_eq!(s.blocks.len(), 2, "empty Enter dropped");
        assert_eq!(s.blocks[0].cmd, "df -h /var");
        assert_eq!(s.blocks[0].output.len(), 2);
        assert_eq!(s.current.as_ref().unwrap().1, "ls -l");
        assert!(!s.running());

        // Output after the last prompt: still running.
        let s = parse(&lines("ops@app-01:~$ sleep 5\n[sudo] password for ops:"), &sig);
        assert!(s.running());
        assert_eq!(s.blocks[0].output, vec!["[sudo] password for ops:"]);
    }

    #[test]
    fn other_hosts_prompts_in_output_are_not_blocks() {
        let sig = Sig::UserHost("me@laptop".into());
        let s = parse(&lines("me@laptop ~ % cat notes\nroot@other:~# not mine\nme@laptop ~ %"), &sig);
        assert_eq!(s.blocks.len(), 1);
        assert_eq!(s.blocks[0].output, vec!["root@other:~# not mine"]);
    }

    #[test]
    fn custom_prompts_use_the_raw_text() {
        assert_eq!(sig_of("❯"), Some(Sig::Raw("❯".into())));
        assert_eq!(sig_of("ops@app-01:~$ "), Some(Sig::UserHost("ops@app-01".into())));
        assert_eq!(sig_of("ops@app-01:~$ typing"), None);
        assert_eq!(sig_of("some output"), None);
        let s = parse(&lines("❯ git status\nclean\n❯"), &Sig::Raw("❯".into()));
        assert_eq!(s.blocks[0].cmd, "git status");
    }

    #[test]
    fn metas_match_newest_first() {
        let b = |c: &str| Block { prompt: parse_prompt("a@b:~$").unwrap().0, cmd: c.into(), output: vec!["x".into()] };
        let metas = vec![Meta::new("ls", Source::User), Meta::new("df -h", Source::Ai), Meta::new("ls", Source::Ai)];
        let m = match_metas(&[b("ls"), b("df -h"), b("ls"), b("pwd")], &metas);
        assert_eq!(m[3].map(|x| x.source), None);
        assert_eq!(m[2].map(|x| x.source), Some(Source::Ai));
        assert_eq!(m[1].map(|x| x.source), Some(Source::Ai));
        assert_eq!(m[0].map(|x| x.source), Some(Source::User));
    }

    #[test]
    fn ghost_prefers_history_then_known_commands() {
        let metas = vec![Meta::new("docker ps -a", Source::User)];
        assert_eq!(ghost("dock", &metas).as_deref(), Some("er ps -a"));
        assert_eq!(ghost("upt", &[]).as_deref(), Some("ime"));
        assert_eq!(ghost("# question", &metas), None);
        assert_eq!(ghost("", &metas), None);
    }

    #[test]
    fn output_highlights() {
        let h = highlight("/dev/sda2 80G 91% /var error", "");
        assert!(h.contains(&("/dev/sda2".into(), Mark::Path)));
        assert!(h.contains(&("91%".into(), Mark::High)));
        assert!(h.contains(&("error".into(), Mark::Err)));
        let h = highlight("Active: active (running) since", "SINCE");
        assert!(h.contains(&("active (running)".into(), Mark::Ok)));
        assert!(h.contains(&("since".into(), Mark::Found)));
        assert_eq!(h.iter().map(|(s, _)| s.as_str()).collect::<String>(), "Active: active (running) since");
    }
}
