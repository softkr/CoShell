//! Local shell transport over a native PTY.

use std::io::{Read, Write};
use std::sync::Mutex;

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::{GridSize, Transport};

/// Fallback locale so wide (CJK) characters are emitted as UTF-8.
const FALLBACK_LANG: &str = "en_US.UTF-8";

pub struct LocalPty {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
}

fn pty_size(size: GridSize) -> PtySize {
    PtySize {
        rows: size.rows.max(1),
        cols: size.cols.max(1),
        pixel_width: size.cols.saturating_mul(size.cell_width),
        pixel_height: size.rows.saturating_mul(size.cell_height),
    }
}

/// Name of the shell a local tab runs (`zsh`, `bash`, `cmd` …) for tab titles and the status bar.
pub fn local_shell_name() -> String {
    // What `CommandBuilder::new_default_prog` starts: $SHELL on Unix, %COMSPEC% on Windows.
    let var = if cfg!(windows) { "COMSPEC" } else { "SHELL" };
    std::env::var_os(var)
        .and_then(|p| std::path::Path::new(&p).file_stem().map(|s| s.to_string_lossy().into_owned()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| if cfg!(windows) { "cmd".into() } else { "sh".into() })
}

impl LocalPty {
    /// Spawn `$SHELL` (login shell) and return the transport plus its output reader.
    pub fn spawn(size: GridSize) -> anyhow::Result<(Self, Box<dyn Read + Send>)> {
        let pair = native_pty_system().openpty(pty_size(size))?;

        let mut cmd = CommandBuilder::new_default_prog();
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "CoShell");
        if std::env::var_os("LANG").is_none() {
            cmd.env("LANG", FALLBACK_LANG);
        }
        if let Some(home) = std::env::home_dir() {
            cmd.cwd(home);
        }

        let child = pair.slave.spawn_command(cmd)?;
        // Close our copy of the slave so EOF is seen when the shell exits.
        drop(pair.slave);

        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        Ok((
            Self {
                master: Mutex::new(pair.master),
                writer: Mutex::new(writer),
                child: Mutex::new(child),
            },
            reader,
        ))
    }
}

impl Transport for LocalPty {
    fn write(&self, bytes: &[u8]) {
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(bytes).and_then(|_| w.flush());
        }
    }

    fn resize(&self, size: GridSize) {
        if let Ok(m) = self.master.lock() {
            let _ = m.resize(pty_size(size));
        }
    }
}

impl Drop for LocalPty {
    fn drop(&mut self) {
        if let Ok(mut c) = self.child.lock() {
            let _ = c.kill();
        }
    }
}
