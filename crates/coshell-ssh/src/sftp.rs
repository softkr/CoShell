//! Files on the target over SFTP: a subsystem channel of the connection that is
//! already open, so jump hosts, host keys and authentication are the session's.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{FileAttributes, FileType};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{Connection, SshError};

/// Bytes moved per read; progress and cancellation are checked between chunks.
const CHUNK: usize = 64 * 1024;
/// Suffix of a transfer in progress, renamed over the real name once complete.
const PART_SUFFIX: &str = ".coshell-part";
/// Permission bits a client may set (no file-type bits).
const MODE_MASK: u32 = 0o7777;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Dir,
    File,
    /// Sockets, devices, and links that point nowhere.
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    /// What the entry is; for a symbolic link, what it points to.
    pub kind: FileKind,
    pub link: bool,
    pub size: u64,
    /// Seconds since the Unix epoch.
    pub modified: Option<u32>,
    /// Unix permission bits (`0o755`).
    pub permissions: Option<u32>,
}

/// Shared between a running transfer and whoever watches it.
#[derive(Clone, Debug, Default)]
pub struct TransferProgress {
    done: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
}

impl TransferProgress {
    pub fn done(&self) -> u64 {
        self.done.load(Ordering::Relaxed)
    }
    /// Bytes to move: the file, or all files of a folder (0 until the transfer has looked).
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }
    /// Ask the transfer to stop; it ends with [`SshError::Cancelled`].
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// An SFTP session on the target. Cheap to clone; requests may run concurrently.
#[derive(Clone)]
pub struct Sftp(Arc<SftpSession>);

fn err(what: &str, e: impl std::fmt::Display) -> SshError {
    SshError::Sftp(format!("{what}: {e}"))
}

/// `dir` + `name` as a remote (POSIX) path.
pub(crate) fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') { format!("{dir}{name}") } else { format!("{dir}/{name}") }
}

fn kind_of(t: FileType) -> FileKind {
    match t {
        FileType::Dir => FileKind::Dir,
        FileType::File => FileKind::File,
        _ => FileKind::Other,
    }
}

impl Connection {
    /// Open the SFTP subsystem on the target (fails when the server has it switched off).
    pub async fn sftp(&self) -> Result<Sftp, SshError> {
        let label = self.labels().last().cloned().unwrap_or_default();
        let ssh = |source| SshError::Ssh { hop: label.clone(), source };
        let mut channel = self.target().channel_open_session().await.map_err(ssh)?;
        channel.request_subsystem(true, "sftp").await.map_err(ssh)?;
        // The answer arrives as a message; without waiting, a refusal would only show as a timeout.
        loop {
            match channel.wait().await {
                Some(russh::ChannelMsg::Success) => break,
                Some(russh::ChannelMsg::Failure) | None => {
                    return Err(SshError::Sftp(if crate::english() { "the server does not offer SFTP".into() } else { "서버가 SFTP를 지원하지 않습니다".into() }));
                }
                Some(_) => {}
            }
        }
        let session = SftpSession::new(channel.into_stream()).await.map_err(|e| err("sftp", e))?;
        Ok(Sftp(Arc::new(session)))
    }
}

impl Sftp {
    /// Absolute form of `path` (`.` is the directory the session starts in).
    pub async fn resolve(&self, path: &str) -> Result<String, SshError> {
        let p = self.0.canonicalize(path).await.map_err(|e| err(path, e))?;
        Ok(p)
    }

    /// Entries of `dir`: folders first, then by name.
    pub async fn list(&self, dir: &str) -> Result<Vec<FileEntry>, SshError> {
        let mut out = Vec::new();
        for e in self.0.read_dir(dir).await.map_err(|e| err(dir, e))? {
            let meta = e.metadata();
            let name = e.file_name();
            let link = meta.file_type().is_symlink();
            // A link is shown as what it leads to, so a linked folder can be opened.
            let kind = if link {
                self.0.metadata(join(dir, &name)).await.map_or(FileKind::Other, |m| kind_of(m.file_type()))
            } else {
                kind_of(meta.file_type())
            };
            out.push(FileEntry { name, kind, link, size: meta.size.unwrap_or(0), modified: meta.mtime, permissions: meta.permissions.map(|p| p & MODE_MASK) });
        }
        out.sort_by_key(|e| (e.kind != FileKind::Dir, e.name.to_lowercase()));
        Ok(out)
    }

    /// Copy the remote file to `local`. Written beside it first, so a failed or
    /// cancelled download never leaves a half file under the real name.
    pub async fn download(&self, remote: &str, local: &Path, progress: &TransferProgress) -> Result<(), SshError> {
        let total = self.0.metadata(remote).await.map_err(|e| err(remote, e))?.size.unwrap_or(0);
        progress.total.store(total, Ordering::Relaxed);
        let mut src = self.0.open(remote).await.map_err(|e| err(remote, e))?;
        let part = part_path(local);
        let result = async {
            let mut dst = tokio::fs::File::create(&part).await.map_err(|e| err(&part.display().to_string(), e))?;
            copy(&mut src, &mut dst, progress, remote).await?;
            dst.flush().await.map_err(|e| err(&part.display().to_string(), e))?;
            drop(dst);
            tokio::fs::rename(&part, local).await.map_err(|e| err(&local.display().to_string(), e))
        }
        .await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&part).await;
        }
        result
    }

    /// Copy `local` to the remote path. Sent under a temporary name and moved into
    /// place at the end, so the file already there survives a failed upload; its
    /// permissions carry over to the new content.
    pub async fn upload(&self, local: &Path, remote: &str, progress: &TransferProgress) -> Result<(), SshError> {
        let total = tokio::fs::metadata(local).await.map_err(|e| err(&local.display().to_string(), e))?.len();
        progress.total.store(total, Ordering::Relaxed);
        self.put(local, remote, progress).await
    }

    /// Copy the local folder and everything in it to `remote`, merging into a folder
    /// already there. Each file is sent as [`Self::upload`] sends one; progress counts
    /// the whole folder. A link to a file sends its content; a link to a folder is
    /// left out, since it could lead back into the folder and never end.
    pub async fn upload_dir(&self, local: &Path, remote: &str, progress: &TransferProgress) -> Result<(), SshError> {
        let (dirs, files) = walk(local, remote).await?;
        progress.total.store(files.iter().map(|(_, _, size)| size).sum(), Ordering::Relaxed);
        for dir in &dirs {
            if progress.is_cancelled() {
                return Err(SshError::Cancelled(remote.to_string()));
            }
            self.ensure_dir(dir).await?;
        }
        for (path, to, _) in &files {
            self.put(path, to, progress).await?;
        }
        Ok(())
    }

    /// A folder at `path`, made unless one is there already.
    async fn ensure_dir(&self, path: &str) -> Result<(), SshError> {
        match self.0.metadata(path).await {
            Ok(m) if m.file_type().is_dir() => Ok(()),
            _ => self.mkdir(path).await,
        }
    }

    /// Send one file's bytes, counted onto `progress` (whose total the caller sets).
    async fn put(&self, local: &Path, remote: &str, progress: &TransferProgress) -> Result<(), SshError> {
        let shown = local.display().to_string();
        let mut src = tokio::fs::File::open(local).await.map_err(|e| err(&shown, e))?;
        let old_mode = self.0.metadata(remote).await.ok().and_then(|m| m.permissions).map(|p| p & MODE_MASK);
        let part = format!("{remote}{PART_SUFFIX}");
        let result = async {
            let mut dst = self.0.create(part.as_str()).await.map_err(|e| err(&part, e))?;
            copy(&mut src, &mut dst, progress, remote).await?;
            dst.shutdown().await.map_err(|e| err(&part, e))?;
            drop(dst);
            if let Some(mode) = old_mode {
                self.chmod(&part, mode).await?;
            }
            self.replace(&part, remote).await
        }
        .await;
        if result.is_err() {
            let _ = self.0.remove_file(part.as_str()).await;
        }
        result
    }

    /// Move `from` onto `to`, replacing what is there.
    async fn replace(&self, from: &str, to: &str) -> Result<(), SshError> {
        // The plain SFTP rename refuses an existing target; OpenSSH's extension does not.
        if self.0.posix_rename(from, to).await.map_err(|e| err(to, e))? {
            return Ok(());
        }
        let _ = self.0.remove_file(to).await;
        self.0.rename(from, to).await.map_err(|e| err(to, e))
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<(), SshError> {
        self.0.rename(from, to).await.map_err(|e| err(from, e))
    }

    pub async fn mkdir(&self, path: &str) -> Result<(), SshError> {
        self.0.create_dir(path).await.map_err(|e| err(path, e))
    }

    pub async fn chmod(&self, path: &str, mode: u32) -> Result<(), SshError> {
        let attrs = FileAttributes { permissions: Some(mode & MODE_MASK), ..FileAttributes::empty() };
        self.0.set_metadata(path, attrs).await.map_err(|e| err(path, e))
    }

    /// Delete a file, or a folder with everything in it. Links are removed, never followed.
    pub async fn remove(&self, path: &str) -> Result<(), SshError> {
        let is_dir = self.0.symlink_metadata(path).await.map_err(|e| err(path, e))?.file_type().is_dir();
        if !is_dir {
            return self.0.remove_file(path).await.map_err(|e| err(path, e));
        }
        // Depth-first without recursion: folders are removed after their contents.
        let mut pending = vec![path.to_string()];
        let mut emptied = Vec::new();
        while let Some(dir) = pending.pop() {
            for e in self.0.read_dir(dir.as_str()).await.map_err(|e| err(&dir, e))? {
                let child = join(&dir, &e.file_name());
                if e.file_type().is_dir() {
                    pending.push(child);
                } else {
                    self.0.remove_file(child.as_str()).await.map_err(|e| err(&child, e))?;
                }
            }
            emptied.push(dir);
        }
        for dir in emptied.iter().rev() {
            self.0.remove_dir(dir.as_str()).await.map_err(|e| err(dir, e))?;
        }
        Ok(())
    }
}

/// Remote folders to make (parents before children) and files to send
/// (local path, remote path, size) for the local folder `local` going to `remote`.
async fn walk(local: &Path, remote: &str) -> Result<(Vec<String>, Vec<(PathBuf, String, u64)>), SshError> {
    let (mut dirs, mut files) = (Vec::new(), Vec::new());
    let mut pending = vec![(local.to_path_buf(), remote.to_string())];
    while let Some((dir, to)) = pending.pop() {
        let shown = dir.display().to_string();
        let mut entries = tokio::fs::read_dir(&dir).await.map_err(|e| err(&shown, e))?;
        while let Some(e) = entries.next_entry().await.map_err(|e| err(&shown, e))? {
            let path = e.path();
            let child = join(&to, &e.file_name().to_string_lossy());
            // The entry itself, not where a link leads.
            if e.file_type().await.map_err(|e| err(&path.display().to_string(), e))?.is_dir() {
                pending.push((path, child));
                continue;
            }
            // Followed here: linked folders, broken links and devices are skipped.
            if let Ok(m) = tokio::fs::metadata(&path).await
                && m.is_file()
            {
                files.push((path, child, m.len()));
            }
        }
        dirs.push(to);
    }
    Ok((dirs, files))
}

fn part_path(local: &Path) -> PathBuf {
    let mut name = local.file_name().unwrap_or_default().to_os_string();
    name.push(PART_SUFFIX);
    local.with_file_name(name)
}

async fn copy<R, W>(src: &mut R, dst: &mut W, progress: &TransferProgress, name: &str) -> Result<(), SshError>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; CHUNK];
    loop {
        if progress.is_cancelled() {
            return Err(SshError::Cancelled(name.to_string()));
        }
        let n = src.read(&mut buf).await.map_err(|e| err(name, e))?;
        if n == 0 {
            return Ok(());
        }
        dst.write_all(&buf[..n]).await.map_err(|e| err(name, e))?;
        progress.done.fetch_add(n as u64, Ordering::Relaxed);
    }
}
