//! Remote files of an SSH tab, over SFTP on the connection the tab already has:
//! browsing, transfers with progress, file management, following the terminal's
//! folder and editing a remote file locally.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use coshell_ssh::{FileEntry, FileKind, Sftp, TransferProgress};
use iced::Task;

use crate::app::{CoShell, Message, SidebarTab, TabId};
use crate::conn::Once;
use crate::i18n::{tf, tr};

/// Longest permission string accepted (`0755`).
const MODE_MAX_LEN: usize = 4;
/// Name a download gets when the server's name cannot be used as a local one.
const FALLBACK_NAME: &str = "download";
/// Folders of the other shells on this SSH connection (Linux), one per line: the
/// terminal's shell is a sibling of this command under the connection's `sshd`.
/// Run through `sh` so a csh login shell does not matter; the login shell itself,
/// when it did not hand over to `sh`, is left out.
const SHELL_DIRS: &str = r#"sh -c 'p=$PPID; case $(cat /proc/$p/comm 2>/dev/null) in sshd*) ;; *) p=$(ps -o ppid= -p $p | tr -d " ");; esac; for c in $(ps -o pid= --ppid $p); do [ "$c" = "$PPID" ] || [ "$c" = "$$" ] || readlink /proc/$c/cwd; done'"#;
/// Bytes of [`SHELL_DIRS`] output read.
const SHELL_DIRS_LIMIT: usize = 4096;
/// How long [`SHELL_DIRS`] may take.
const SHELL_DIRS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The tab's SFTP session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Link {
    /// Not asked for yet (opened when the file panel is first shown).
    #[default]
    Idle,
    Opening,
    Ready,
    /// The server has no SFTP, or opening it failed.
    Unavailable(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransferState {
    Running,
    Done,
    Cancelled,
    Failed(String),
}

/// What happens once a transfer has finished.
#[derive(Clone, Debug)]
enum After {
    Nothing,
    /// An upload into this folder: list it again if it is still shown.
    Refresh(String),
    /// A file fetched for editing: open it and watch it for changes.
    Edit { remote: String, local: PathBuf },
}

pub struct Transfer {
    pub id: u64,
    pub name: String,
    pub upload: bool,
    pub progress: TransferProgress,
    pub state: TransferState,
    after: After,
}

impl Transfer {
    /// 0.0–1.0; an empty file counts as done.
    pub fn fraction(&self) -> f32 {
        match self.progress.total() {
            0 => f32::from(u8::from(self.state == TransferState::Done)),
            total => (self.progress.done() as f32 / total as f32).min(1.0),
        }
    }
}

/// A remote file opened in a local editor.
pub struct RemoteEdit {
    pub remote: String,
    pub local: PathBuf,
    /// The local copy's modification time when it was last in step with the server.
    seen: Option<SystemTime>,
    /// Saved locally since then: offered for upload.
    pub changed: bool,
}

impl RemoteEdit {
    pub fn name(&self) -> &str {
        self.remote.rsplit('/').next().unwrap_or(&self.remote)
    }
}

/// A checked request from one of the panel's questions.
enum Op {
    Mkdir(String),
    Rename(String, String),
    Chmod(String, u32),
}

/// A one-line question in the panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileInput {
    NewFolder(String),
    Rename { from: String, to: String },
    /// Octal permissions (`644`).
    Chmod { name: String, mode: String },
}

/// Something that cannot be undone, waiting for a yes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileConfirm {
    Delete { name: String, folder: bool },
    /// Uploads that would replace files already in the folder (or merge into a folder there).
    Overwrite(Vec<PathBuf>),
}

#[derive(Default)]
pub struct Files {
    pub link: Link,
    sftp: Option<Sftp>,
    /// Where the session starts (the user's home on most servers).
    pub home: String,
    /// The folder shown (absolute).
    pub path: String,
    pub entries: Vec<FileEntry>,
    pub loading: bool,
    pub error: Option<String>,
    /// The file whose actions are shown.
    pub selected: Option<String>,
    /// Entry whose right-click menu is open.
    pub menu: Option<String>,
    pub transfers: Vec<Transfer>,
    seq: u64,
    pub input: Option<FileInput>,
    pub confirm: Option<FileConfirm>,
    /// Show the folder the terminal is in.
    pub follow: bool,
    /// The prompt's folder (as the prompt shows it) last acted on, so a folder is followed once.
    followed: Option<String>,
    pub edits: Vec<RemoteEdit>,
    /// When the folder shown last changed, and from which side its rows come in (1: deeper,
    /// from the right; -1: back up, from the left).
    pub moved: Option<(std::time::Instant, f32)>,
    /// The entry that was just uploaded or downloaded, and when: it glows a moment.
    pub flash: Option<(String, std::time::Instant)>,
}

/// How long a just-transferred entry glows (s).
const FLASH: f32 = 1.4;

impl Files {
    /// Seconds since the folder shown changed, and the side it came in from, while its rows
    /// are still coming in.
    pub fn moved_since(&self) -> Option<(f32, f32)> {
        #[cfg(test)]
        return None;
        #[cfg(not(test))]
        self.moved.map(|(at, side)| (at.elapsed().as_secs_f32(), side)).filter(|(e, _)| *e < crate::motion::SLIDE_LEN)
    }

    /// How brightly entry `name` glows after its transfer (1 → 0).
    pub fn flash_of(&self, name: &str) -> f32 {
        self.flash.as_ref().filter(|(n, _)| n == name).map_or(0.0, |(_, at)| (1.0 - at.elapsed().as_secs_f32() / FLASH).max(0.0).powi(2))
    }

    /// Something in the panel is moving (rows coming in, an entry glowing).
    pub fn animating(&self) -> bool {
        self.moved_since().is_some() || self.flash.as_ref().is_some_and(|(_, at)| at.elapsed().as_secs_f32() < FLASH)
    }

    /// The connection is gone: forget the session and what it showed, keep the preference.
    pub fn reset(&mut self) {
        for t in &self.transfers {
            t.progress.cancel();
        }
        *self = Self { follow: self.follow, ..Self::default() };
    }

    pub fn running(&self) -> usize {
        self.transfers.iter().filter(|t| t.state == TransferState::Running).count()
    }

    /// Something Esc would close.
    pub fn has_popup(&self) -> bool {
        self.confirm.is_some() || self.menu.is_some() || self.input.is_some()
    }

    pub fn entry(&self, name: &str) -> Option<&FileEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// A ready session showing `path` (tests).
    #[cfg(test)]
    pub fn fake(path: &str, entries: Vec<FileEntry>) -> Self {
        Self { link: Link::Ready, home: path.into(), path: path.into(), entries, ..Self::default() }
    }

    /// A transfer as the panel would show it mid-way (tests).
    #[cfg(test)]
    pub fn fake_transfer(&mut self, name: &str, upload: bool, state: TransferState) {
        self.seq += 1;
        self.transfers.push(Transfer { id: self.seq, name: name.into(), upload, progress: TransferProgress::default(), state, after: After::Nothing });
    }
}

#[derive(Debug, Clone)]
pub enum FilesMsg {
    Opened(Once<Result<(Sftp, String), String>>),
    Retry,
    /// Show this folder (absolute path).
    Go(String),
    Up,
    Refresh,
    Listed(String, Result<Vec<FileEntry>, String>),
    Select(String),
    Menu(Option<String>),
    /// Pick local files to send into the folder shown.
    Upload,
    UploadPicked(Vec<PathBuf>),
    /// Ask where to save this file.
    Download(String),
    DownloadTo(String, Option<PathBuf>),
    TransferDone(u64, Result<(), String>),
    TransferCancel(u64),
    /// Drop finished transfers from the list.
    TransfersClear,
    /// Redraw while bytes move.
    Tick,
    NewFolder,
    RenameAsk(String),
    ChmodAsk(String),
    InputChanged(String),
    InputSubmit,
    InputCancel,
    DeleteAsk(String),
    ConfirmYes,
    ConfirmNo,
    OpDone(Result<(), String>),
    Follow(bool),
    /// Look where the terminal is now.
    FollowTick,
    /// The prompt showed only this folder name; the shells' folders the server reported.
    FollowFound(String, Result<String, String>),
    /// Fetch this file and open it in a local editor.
    Edit(String),
    /// Look for local saves of the files being edited.
    EditScan,
    EditUpload(usize),
    EditClose(usize),
    /// Esc: close the question, menu or input that is open.
    Dismiss,
}

/// `dir` + `name` as a remote path.
pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') { format!("{dir}{name}") } else { format!("{dir}/{name}") }
}

/// The folder above `path` (`/` is its own parent).
pub fn parent(path: &str) -> String {
    match path.trim_end_matches('/').rsplit_once('/') {
        Some(("", _)) | None => "/".into(),
        Some((up, _)) => up.into(),
    }
}

/// A remote name as a local file name: the server chooses remote names, so path
/// separators and anything that could climb out of the target folder are taken out.
pub fn safe_name(name: &str) -> String {
    let clean: String = name.chars().map(|c| if matches!(c, '/' | '\\' | ':') || c.is_control() { '_' } else { c }).collect();
    let clean = clean.trim().trim_matches('.').trim().to_string();
    if clean.is_empty() { FALLBACK_NAME.into() } else { clean }
}

/// A name typed for a new folder or a rename: one path component.
fn check_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.chars().any(char::is_control) {
        return Err(tr("이름에 / 나 제어 문자는 쓸 수 없고, 비워 둘 수 없습니다").into());
    }
    Ok(name)
}

/// `644` / `0755` → permission bits.
fn parse_mode(mode: &str) -> Result<u32, String> {
    let mode = mode.trim();
    let ok = (3..=MODE_MAX_LEN).contains(&mode.len()) && mode.chars().all(|c| ('0'..='7').contains(&c));
    ok.then(|| u32::from_str_radix(mode, 8).ok()).flatten().ok_or_else(|| tr("권한은 644, 755 같은 8진수로 입력하세요").to_string())
}

/// The terminal prompt's folder as an absolute path, when it says enough (`~`, `~/x`, `/x`).
pub fn prompt_dir(cwd: &str, home: &str) -> Option<String> {
    match cwd {
        "~" => Some(home.to_string()),
        c if c.starts_with("~/") => Some(join(home, &c[2..])),
        c if c.starts_with('/') => Some(c.to_string()),
        // `[user@host dir]$` shows the last component only: not enough to go by.
        _ => None,
    }
}

/// The one folder named `name` among the [`SHELL_DIRS`] output, if exactly one.
fn shell_dir(name: &str, out: &str) -> Option<String> {
    let mut found: Vec<&str> = out.lines().filter(|l| l.starts_with('/') && (*l == name || l.rsplit('/').next() == Some(name))).collect();
    found.sort_unstable();
    found.dedup();
    match found.as_slice() {
        [one] => Some(one.to_string()),
        _ => None,
    }
}

/// Open a file for editing. Never "run" it: a downloaded script must not start
/// just because its type has a launcher.
fn open_in_editor(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg("-t");
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = std::process::Command::new("notepad");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(path).spawn().map(|_| ())
}

/// Put the cursor in the panel's question, with what is there selected.
fn focus_input() -> Task<Message> {
    let id = crate::ui_files::FILE_INPUT;
    Task::batch([iced::widget::operation::focus(id), iced::widget::operation::select_all(id)])
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl CoShell {
    fn files_mut(&mut self, id: TabId) -> Option<&mut Files> {
        self.tabs.iter_mut().find(|t| t.id == id).map(|t| &mut t.files)
    }

    /// Open SFTP for the active tab once its file panel is on screen.
    pub(crate) fn ensure_files(&mut self) -> Task<Message> {
        if self.sidebar_tab != SidebarTab::Files {
            return Task::none();
        }
        let Some(tab) = self.active.and_then(|id| self.tabs.iter_mut().find(|t| t.id == id)) else { return Task::none() };
        let (id, Some(conn)) = (tab.id, tab.conn.clone()) else { return Task::none() };
        if tab.files.link != Link::Idle {
            return Task::none();
        }
        tab.files.link = Link::Opening;
        Task::perform(
            async move {
                let sftp = conn.sftp().await.map_err(|e| e.to_string())?;
                let home = sftp.resolve(".").await.map_err(|e| e.to_string())?;
                Ok((sftp, home))
            },
            move |r| Message::Files(id, FilesMsg::Opened(Once::new(r))),
        )
    }

    fn list_files(&mut self, id: TabId, path: String) -> Task<Message> {
        let Some(f) = self.files_mut(id) else { return Task::none() };
        let Some(sftp) = f.sftp.clone() else { return Task::none() };
        f.loading = true;
        f.error = None;
        Task::perform(
            async move {
                let r = sftp.list(&path).await.map_err(|e| e.to_string());
                (path, r)
            },
            move |(path, r)| Message::Files(id, FilesMsg::Listed(path, r)),
        )
    }

    fn start_transfer(&mut self, id: TabId, name: String, upload: bool, local: PathBuf, remote: String, after: After) -> Task<Message> {
        let Some(f) = self.files_mut(id) else { return Task::none() };
        let Some(sftp) = f.sftp.clone() else { return Task::none() };
        f.seq += 1;
        let tid = f.seq;
        let progress = TransferProgress::default();
        f.transfers.push(Transfer { id: tid, name, upload, progress: progress.clone(), state: TransferState::Running, after });
        Task::perform(
            async move {
                let r = match upload {
                    true if local.is_dir() => sftp.upload_dir(&local, &remote, &progress).await,
                    true => sftp.upload(&local, &remote, &progress).await,
                    false => sftp.download(&remote, &local, &progress).await,
                };
                r.map_err(|e| e.to_string())
            },
            move |r| Message::Files(id, FilesMsg::TransferDone(tid, r)),
        )
    }

    /// Send local files and folders into the folder shown.
    fn start_uploads(&mut self, id: TabId, paths: Vec<PathBuf>) -> Task<Message> {
        let Some(dir) = self.files_mut(id).map(|f| f.path.clone()) else { return Task::none() };
        let tasks: Vec<_> = paths
            .into_iter()
            .filter_map(|p| {
                let name = p.file_name()?.to_string_lossy().into_owned();
                let remote = join(&dir, &name);
                Some(self.start_transfer(id, name, true, p, remote, After::Refresh(dir.clone())))
            })
            .collect();
        Task::batch(tasks)
    }

    /// Files and folders dropped on the window go to the active tab's folder.
    pub(crate) fn drop_file(&mut self, path: PathBuf) -> Task<Message> {
        match self.active.filter(|id| self.tabs.iter().any(|t| t.id == *id && t.files.link == Link::Ready)) {
            Some(id) if self.sidebar_tab == SidebarTab::Files => self.update_files(id, FilesMsg::UploadPicked(vec![path])),
            _ => {
                self.notice = Some(tr("파일을 올리려면 SSH 탭에서 사이드바의 ‘파일’을 여세요").into());
                Task::none()
            }
        }
    }

    /// The folder the terminal's prompt shows for `id` (`~/x`, `/x`, or just a name).
    fn terminal_cwd(&self, id: TabId) -> Option<String> {
        let tab = self.tabs.iter().find(|t| t.id == id)?;
        let (prompt, _) = self.screen_of(tab)?.current?;
        Some(prompt.cwd).filter(|c| !c.is_empty())
    }

    /// Show the folder the prompt shows as `cwd`. A bare name is looked up on the server.
    fn follow_to(&mut self, id: TabId, cwd: String) -> Task<Message> {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else { return Task::none() };
        tab.files.followed = Some(cwd.clone());
        if let Some(dir) = prompt_dir(&cwd, &tab.files.home) {
            return if dir == tab.files.path { Task::none() } else { self.list_files(id, dir) };
        }
        let Some(conn) = tab.conn.clone() else { return Task::none() };
        Task::perform(
            async move {
                let run = conn.exec(SHELL_DIRS, SHELL_DIRS_LIMIT);
                match tokio::time::timeout(SHELL_DIRS_TIMEOUT, run).await {
                    Ok(Ok(out)) => Ok(String::from_utf8_lossy(&out.output).into_owned()),
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(_) => Err("timeout".to_string()),
                }
            },
            move |r| Message::Files(id, FilesMsg::FollowFound(cwd, r)),
        )
    }

    pub(crate) fn update_files(&mut self, id: TabId, m: FilesMsg) -> Task<Message> {
        // Read before the panel is borrowed for writing.
        let terminal_cwd = matches!(m, FilesMsg::FollowTick | FilesMsg::Follow(true)).then(|| self.terminal_cwd(id)).flatten();
        let Some(f) = self.files_mut(id) else { return Task::none() };
        match m {
            FilesMsg::Opened(r) => match r.take() {
                Some(Ok((sftp, home))) => {
                    f.sftp = Some(sftp);
                    f.link = Link::Ready;
                    f.home = home.clone();
                    return self.list_files(id, home);
                }
                Some(Err(e)) => f.link = Link::Unavailable(e),
                None => {}
            },
            FilesMsg::Retry => {
                f.reset();
                return self.ensure_files();
            }
            FilesMsg::Go(path) => return self.list_files(id, path),
            FilesMsg::Up => {
                let up = parent(&f.path);
                return self.list_files(id, up);
            }
            FilesMsg::Refresh => {
                let path = f.path.clone();
                return self.list_files(id, path);
            }
            FilesMsg::Listed(path, r) => {
                f.loading = false;
                match r {
                    Ok(entries) => {
                        if f.path != path {
                            f.selected = None;
                            f.input = None;
                            // Deeper in comes from the right, back out from the left.
                            let deeper = path.starts_with(&format!("{}/", f.path.trim_end_matches('/')));
                            f.moved = Some((std::time::Instant::now(), if deeper { 1.0 } else { -1.0 }));
                        }
                        f.path = path;
                        f.entries = entries;
                    }
                    // The folder shown stays; only the attempt is reported.
                    Err(e) => f.error = Some(e),
                }
            }
            FilesMsg::Select(name) => {
                f.menu = None;
                f.selected = (f.selected.as_deref() != Some(name.as_str())).then_some(name);
            }
            FilesMsg::Menu(name) => f.menu = name,
            FilesMsg::Upload => {
                return Task::perform(
                    async move {
                        let picked = rfd::AsyncFileDialog::new().pick_files().await.unwrap_or_default();
                        picked.iter().map(|p| p.path().to_path_buf()).collect::<Vec<_>>()
                    },
                    move |paths| Message::Files(id, FilesMsg::UploadPicked(paths)),
                );
            }
            FilesMsg::UploadPicked(paths) => {
                // A folder over one already there merges into it, replacing same-named files.
                let clash = paths.iter().any(|p| p.file_name().is_some_and(|n| f.entry(&n.to_string_lossy()).is_some()));
                if clash {
                    f.confirm = Some(FileConfirm::Overwrite(paths));
                } else if !paths.is_empty() {
                    return self.start_uploads(id, paths);
                }
            }
            FilesMsg::Download(name) => {
                f.menu = None;
                let suggested = safe_name(&name);
                return Task::perform(
                    async move { rfd::AsyncFileDialog::new().set_file_name(suggested).save_file().await.map(|h| h.path().to_path_buf()) },
                    move |to| Message::Files(id, FilesMsg::DownloadTo(name.clone(), to)),
                );
            }
            FilesMsg::DownloadTo(name, Some(local)) => {
                let remote = join(&f.path, &name);
                return self.start_transfer(id, name, false, local, remote, After::Nothing);
            }
            FilesMsg::DownloadTo(_, None) => {}
            FilesMsg::TransferDone(tid, r) => {
                let Some(t) = f.transfers.iter_mut().find(|t| t.id == tid) else { return Task::none() };
                t.state = match r {
                    Ok(()) => TransferState::Done,
                    Err(_) if t.progress.is_cancelled() => TransferState::Cancelled,
                    Err(e) => TransferState::Failed(e),
                };
                if t.state != TransferState::Done {
                    return Task::none();
                }
                // What arrived (or left) lights up in the list.
                let name = t.name.clone();
                f.flash = Some((name, std::time::Instant::now()));
                let Some(t) = f.transfers.iter_mut().find(|t| t.id == tid) else { return Task::none() };
                match std::mem::replace(&mut t.after, After::Nothing) {
                    After::Refresh(dir) if dir == f.path => return self.list_files(id, dir),
                    After::Edit { remote, local } => {
                        if let Err(e) = open_in_editor(&local) {
                            self.notice = Some(tf("편집기를 열 수 없습니다: {0}", &[&e]));
                            return Task::none();
                        }
                        let seen = modified(&local);
                        if let Some(f) = self.files_mut(id) {
                            f.edits.retain(|e| e.remote != remote);
                            f.edits.push(RemoteEdit { remote, local, seen, changed: false });
                        }
                    }
                    _ => {}
                }
            }
            FilesMsg::TransferCancel(tid) => {
                if let Some(t) = f.transfers.iter().find(|t| t.id == tid) {
                    t.progress.cancel();
                }
            }
            FilesMsg::TransfersClear => f.transfers.retain(|t| t.state == TransferState::Running),
            FilesMsg::Tick => {}
            FilesMsg::NewFolder => {
                f.input = Some(FileInput::NewFolder(String::new()));
                return focus_input();
            }
            FilesMsg::RenameAsk(name) => {
                f.menu = None;
                f.input = Some(FileInput::Rename { to: name.clone(), from: name });
                return focus_input();
            }
            FilesMsg::ChmodAsk(name) => {
                f.menu = None;
                let mode = f.entry(&name).and_then(|e| e.permissions).map(|p| format!("{p:o}")).unwrap_or_default();
                f.input = Some(FileInput::Chmod { name, mode });
                return focus_input();
            }
            FilesMsg::Dismiss => {
                if f.confirm.take().is_none() && f.menu.take().is_none() {
                    f.input = None;
                }
            }
            FilesMsg::InputChanged(v) => match &mut f.input {
                Some(FileInput::NewFolder(name)) => *name = v,
                Some(FileInput::Rename { to, .. }) => *to = v,
                Some(FileInput::Chmod { mode, .. }) => *mode = v.chars().take(MODE_MAX_LEN).collect(),
                None => {}
            },
            FilesMsg::InputCancel => f.input = None,
            FilesMsg::InputSubmit => {
                let Some(input) = f.input.clone() else { return Task::none() };
                let dir = f.path.clone();
                // Checked before anything is sent to the server.
                let op = match input {
                    FileInput::NewFolder(name) => check_name(&name).map(|n| Op::Mkdir(join(&dir, n))),
                    FileInput::Rename { from, to } => check_name(&to).map(|n| Op::Rename(join(&dir, &from), join(&dir, n))),
                    FileInput::Chmod { name, mode } => parse_mode(&mode).map(|m| Op::Chmod(join(&dir, &name), m)),
                };
                let op = match op {
                    Ok(op) => op,
                    Err(e) => {
                        f.error = Some(e);
                        return Task::none();
                    }
                };
                f.input = None;
                f.error = None;
                let Some(sftp) = f.sftp.clone() else { return Task::none() };
                return Task::perform(
                    async move {
                        let r = match op {
                            Op::Mkdir(path) => sftp.mkdir(&path).await,
                            Op::Rename(from, to) if from == to => Ok(()),
                            Op::Rename(from, to) => sftp.rename(&from, &to).await,
                            Op::Chmod(path, mode) => sftp.chmod(&path, mode).await,
                        };
                        r.map_err(|e| e.to_string())
                    },
                    move |r| Message::Files(id, FilesMsg::OpDone(r)),
                );
            }
            FilesMsg::DeleteAsk(name) => {
                f.menu = None;
                let folder = f.entry(&name).is_some_and(|e| e.kind == FileKind::Dir && !e.link);
                f.confirm = Some(FileConfirm::Delete { name, folder });
            }
            FilesMsg::ConfirmNo => f.confirm = None,
            FilesMsg::ConfirmYes => match f.confirm.take() {
                Some(FileConfirm::Overwrite(paths)) => return self.start_uploads(id, paths),
                Some(FileConfirm::Delete { name, .. }) => {
                    let Some(sftp) = f.sftp.clone() else { return Task::none() };
                    let path = join(&f.path, &name);
                    return Task::perform(async move { sftp.remove(&path).await.map_err(|e| e.to_string()) }, move |r| Message::Files(id, FilesMsg::OpDone(r)));
                }
                None => {}
            },
            FilesMsg::OpDone(r) => {
                if let Err(e) = r {
                    f.error = Some(e);
                    return Task::none();
                }
                let path = f.path.clone();
                return self.list_files(id, path);
            }
            FilesMsg::Follow(on) => {
                f.follow = on;
                f.followed = None;
                if let (true, Some(cwd)) = (on, terminal_cwd) {
                    return self.follow_to(id, cwd);
                }
            }
            FilesMsg::FollowTick => {
                // Acted on once per change, so browsing elsewhere is not undone every second.
                if let (true, Link::Ready, Some(cwd)) = (f.follow, &f.link, terminal_cwd)
                    && f.followed.as_deref() != Some(cwd.as_str())
                {
                    return self.follow_to(id, cwd);
                }
            }
            FilesMsg::FollowFound(name, r) => {
                // Still following, and the prompt has not moved on meanwhile.
                let dir = r.ok().and_then(|out| shell_dir(&name, &out));
                if let (true, Some(dir)) = (f.follow && f.followed.as_deref() == Some(name.as_str()), dir)
                    && dir != f.path
                {
                    return self.list_files(id, dir);
                }
            }
            FilesMsg::Edit(name) => {
                f.menu = None;
                f.seq += 1;
                // Its own folder per edit: same-named files from different places do not collide.
                let dir = std::env::temp_dir().join("coshell-edit").join(format!("{}-{id}-{}", std::process::id(), f.seq));
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    self.notice = Some(tf("임시 폴더를 만들 수 없습니다: {0}", &[&e]));
                    return Task::none();
                }
                let local = dir.join(safe_name(&name));
                let remote = join(&f.path, &name);
                return self.start_transfer(id, name, false, local.clone(), remote.clone(), After::Edit { remote, local });
            }
            FilesMsg::EditScan => {
                for e in &mut f.edits {
                    let now = modified(&e.local);
                    if now.is_some() && now != e.seen {
                        e.changed = true;
                    }
                }
            }
            FilesMsg::EditUpload(i) => {
                let Some(e) = f.edits.get_mut(i) else { return Task::none() };
                e.seen = modified(&e.local);
                e.changed = false;
                let (name, local, remote) = (e.name().to_string(), e.local.clone(), e.remote.clone());
                let dir = parent(&remote);
                return self.start_transfer(id, name, true, local, remote, After::Refresh(dir));
            }
            FilesMsg::EditClose(i) => {
                if i < f.edits.len() {
                    let e = f.edits.remove(i);
                    // Only the copy made for this edit, in the folder made for it.
                    let _ = std::fs::remove_file(&e.local);
                    if let Some(dir) = e.local.parent() {
                        let _ = std::fs::remove_dir(dir);
                    }
                }
            }
        }
        Task::none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_paths() {
        assert_eq!(join("/", "etc"), "/etc");
        assert_eq!(join("/var/log", "nginx"), "/var/log/nginx");
        assert_eq!(parent("/var/log"), "/var");
        assert_eq!(parent("/var/"), "/");
        assert_eq!(parent("/"), "/");
    }

    #[test]
    fn server_names_cannot_leave_the_download_folder() {
        assert_eq!(safe_name("report.pdf"), "report.pdf");
        assert_eq!(safe_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_name("..\\..\\evil.bat"), "_.._evil.bat");
        assert_eq!(safe_name(".."), FALLBACK_NAME);
        assert_eq!(safe_name("a\nb"), "a_b");
        assert!(!safe_name("C:\\x").contains([':', '\\']));
    }

    #[test]
    fn typed_names_and_modes_are_checked() {
        assert_eq!(check_name(" logs "), Ok("logs"));
        for bad in ["", "..", "a/b", "a\nb"] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
        assert_eq!(parse_mode("644"), Ok(0o644));
        assert_eq!(parse_mode("0755"), Ok(0o755));
        for bad in ["", "8", "64", "rwx", "07777"] {
            assert!(parse_mode(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn prompt_folder_needs_a_full_path() {
        assert_eq!(prompt_dir("~", "/home/ops").as_deref(), Some("/home/ops"));
        assert_eq!(prompt_dir("~/app/logs", "/home/ops").as_deref(), Some("/home/ops/app/logs"));
        assert_eq!(prompt_dir("/var/log", "/home/ops").as_deref(), Some("/var/log"));
        assert_eq!(prompt_dir("logs", "/home/ops"), None);
    }

    #[test]
    fn a_bare_folder_name_is_found_among_the_shells_folders() {
        // The terminal's shell in /root/Veeam_Agent_DeploymentKit, another session in /root.
        let out = "/root/Veeam_Agent_DeploymentKit\n/root\n";
        assert_eq!(shell_dir("Veeam_Agent_DeploymentKit", out).as_deref(), Some("/root/Veeam_Agent_DeploymentKit"));
        assert_eq!(shell_dir("Veeam_Agent_DeploymentKit", "/root/Veeam_Agent_DeploymentKit\n/root/Veeam_Agent_DeploymentKit\n").as_deref(), Some("/root/Veeam_Agent_DeploymentKit"), "same folder twice");
        assert_eq!(shell_dir("/", "/\n").as_deref(), Some("/"));
        assert_eq!(shell_dir("logs", "/srv/a/logs\n/srv/b/logs\n"), None, "two shells, two such folders: not guessed");
        assert_eq!(shell_dir("logs", "/root\n"), None);
        assert_eq!(shell_dir("logs", "readlink: permission denied\n"), None);
    }
}
