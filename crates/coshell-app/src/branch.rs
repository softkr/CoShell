//! The git branch of the folder the terminal is in, for the prompt's segments.
//!
//! Nothing is run in the folder: the branch is read from `.git/HEAD` (in the folder
//! or one above it), so a repository cannot make this do anything.

use std::time::Duration;

/// How often the prompt is looked at for a change of folder or a finished command.
pub const EVERY: Duration = Duration::from_secs(1);
/// Bytes of [`SCRIPT`] output read, and how long it may take.
pub const LIMIT: usize = 8192;
pub const TIMEOUT: Duration = Duration::from_secs(5);
/// Characters of a commit shown when no branch is checked out.
const SHORT_COMMIT: usize = 7;

/// For a Linux server: `$HOME`, then one line per other shell on this SSH connection,
/// `folder<TAB>first line of its .git/HEAD` (empty outside a repository). The terminal's
/// shell is a sibling of this command under the connection's `sshd`. Run through `sh` so a
/// csh login shell does not matter; the login shell itself, when it did not hand over to
/// `sh`, is left out.
pub const SCRIPT: &str = r#"sh -c 'p=$PPID; case $(cat /proc/$p/comm 2>/dev/null) in sshd*) ;; *) p=$(ps -o ppid= -p $p | tr -d " ");; esac; echo "$HOME"; for c in $(ps -o pid= --ppid $p); do [ "$c" = "$PPID" ] || [ "$c" = "$$" ] || { d=$(readlink /proc/$c/cwd) || continue; g=$d; h=; while :; do if [ -f "$g/.git/HEAD" ]; then read -r h < "$g/.git/HEAD"; break; fi; [ -z "$g" ] && break; g=${g%/*}; done; printf "%s\t%s\n" "$d" "$h"; }; done'"#;

/// What the prompt's branch segment shows for a tab.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// The prompt folder and the number of commands run when the branch was last asked for.
    pub asked: Option<Key>,
    pub branch: Option<String>,
}

/// (folder as the prompt shows it, commands run so far): a new one means look again.
pub type Key = (String, usize);

/// A `.git/HEAD` line as a name: the branch, or a short commit when none is checked out.
pub fn head_name(head: &str) -> Option<String> {
    let head = head.trim();
    if let Some(r) = head.strip_prefix("ref:") {
        let r = r.trim();
        return Some(r.strip_prefix("refs/heads/").unwrap_or(r).to_string()).filter(|b| !b.is_empty());
    }
    (head.len() >= SHORT_COMMIT && head.chars().all(|c| c.is_ascii_hexdigit())).then(|| head[..SHORT_COMMIT].to_string())
}

/// The branch of the folder the prompt shows as `cwd`, from [`SCRIPT`]'s output.
/// A prompt that shows only the folder's name is matched by that name, if only one shell is in such a folder.
pub fn remote(cwd: &str, out: &str) -> Option<String> {
    let mut lines = out.lines();
    let home = lines.next()?;
    let full = crate::files::prompt_dir(cwd, home);
    let mut found: Vec<(&str, &str)> = lines
        .filter_map(|l| l.split_once('\t'))
        .filter(|(dir, _)| match &full {
            Some(full) => dir == full,
            None => dir.rsplit('/').next() == Some(cwd),
        })
        .collect();
    found.sort_unstable();
    found.dedup();
    match found.as_slice() {
        [(_, head)] => head_name(head),
        _ => None,
    }
}

/// The branch of a folder on this computer.
pub async fn local(dir: std::path::PathBuf) -> Option<String> {
    tokio::task::spawn_blocking(move || dir.ancestors().find_map(|d| std::fs::read_to_string(d.join(".git/HEAD")).ok()).and_then(|h| head_name(h.lines().next()?)))
        .await
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_is_a_branch_or_a_short_commit() {
        assert_eq!(head_name("ref: refs/heads/master\n").as_deref(), Some("master"));
        assert_eq!(head_name("ref: refs/heads/feat/login").as_deref(), Some("feat/login"));
        assert_eq!(head_name("4f2a9c1e8b7d6a5f4e3d2c1b0a9f8e7d6c5b4a39").as_deref(), Some("4f2a9c1"));
        assert_eq!(head_name(""), None);
        assert_eq!(head_name("not a head"), None);
    }

    #[test]
    fn branch_of_the_folder_the_prompt_shows() {
        // The terminal's shell in a repository, the file panel's helper in the home folder.
        let out = "/root\n/root/src\tref: refs/heads/master\n/root\t\n";
        assert_eq!(remote("~/src", out).as_deref(), Some("master"));
        assert_eq!(remote("/root/src", out).as_deref(), Some("master"));
        assert_eq!(remote("src", out).as_deref(), Some("master"), "a prompt showing only the folder's name");
        assert_eq!(remote("~", out), None, "not a repository");
        assert_eq!(remote("/etc", out), None, "no shell there");
        // Two shells in different folders of the same name: not guessed.
        assert_eq!(remote("src", "/root\n/a/src\tref: refs/heads/a\n/b/src\tref: refs/heads/b\n"), None);
        // Two shells in the same folder are one answer.
        assert_eq!(remote("src", "/root\n/a/src\tref: refs/heads/a\n/a/src\tref: refs/heads/a\n").as_deref(), Some("a"));
        assert_eq!(remote("~", ""), None);
    }

    #[tokio::test]
    async fn local_branch_is_read_from_the_folder_or_one_above() {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(repo.path().join(".git")).unwrap();
        std::fs::create_dir_all(repo.path().join("a/b")).unwrap();
        std::fs::write(repo.path().join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        assert_eq!(local(repo.path().join("a/b")).await.as_deref(), Some("main"));
        let plain = tempfile::tempdir().unwrap();
        // Outside a repository (unless the temporary folder itself sits in one, which it does not here).
        assert_eq!(local(plain.path().join("nope")).await, None);
    }
}
