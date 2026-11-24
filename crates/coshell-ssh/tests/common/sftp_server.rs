//! An SSH server whose SFTP subsystem serves a local folder as `/`.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use russh::keys::PublicKey;
use russh::server::{self, Msg, Server as _, Session as ServerSession};
use russh::{Channel, ChannelId, MethodKind, MethodSet};
use russh_sftp::protocol::{Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, SftpHandle, SftpString, Status, StatusCode, Version};
use tokio::net::TcpListener;

use super::{TARGET_USER, host_key};

struct SftpServer {
    root: PathBuf,
    /// `false`: the subsystem request is refused, like a server with SFTP switched off.
    enabled: bool,
    channels: HashMap<ChannelId, Channel<Msg>>,
}

impl server::Server for SftpServer {
    type Handler = Self;
    fn new_client(&mut self, _: Option<SocketAddr>) -> Self {
        Self { root: self.root.clone(), enabled: self.enabled, channels: HashMap::new() }
    }
}

impl server::Handler for SftpServer {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, pw: &str) -> Result<server::Auth, Self::Error> {
        Ok(if (user, pw) == TARGET_USER { server::Auth::Accept } else { server::Auth::reject() })
    }

    async fn channel_open_session(&mut self, channel: Channel<Msg>, reply: server::ChannelOpenHandle, _: &mut ServerSession) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn subsystem_request(&mut self, id: ChannelId, name: &str, session: &mut ServerSession) -> Result<(), Self::Error> {
        match self.channels.remove(&id).filter(|_| name == "sftp" && self.enabled) {
            Some(channel) => {
                session.channel_success(id)?;
                russh_sftp::server::run(channel.into_stream(), Files { root: self.root.clone(), handles: HashMap::new(), next: 0 }).await;
            }
            None => session.channel_failure(id)?,
        }
        Ok(())
    }
}

enum Open {
    File(std::fs::File),
    /// Entries not sent yet.
    Dir(Option<Vec<File>>),
}

struct Files {
    root: PathBuf,
    handles: HashMap<String, Open>,
    next: u32,
}

fn ok(id: u32) -> Status {
    Status { id, status_code: StatusCode::Ok, error_message: "Ok".into(), language_tag: "en-US".into() }
}

fn code(e: std::io::Error) -> StatusCode {
    match e.kind() {
        std::io::ErrorKind::NotFound => StatusCode::NoSuchFile,
        std::io::ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
        _ => StatusCode::Failure,
    }
}

impl Files {
    fn local(&self, path: &SftpString) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
    }
    fn handle(&mut self, id: u32, open: Open) -> Handle {
        self.next += 1;
        let key = self.next.to_string();
        self.handles.insert(key.clone(), open);
        Handle { id, handle: key.into() }
    }
    fn key(handle: &SftpHandle) -> String {
        String::from_utf8_lossy(handle.as_bytes()).into_owned()
    }
    fn file(&mut self, handle: &SftpHandle) -> Result<&mut std::fs::File, StatusCode> {
        match self.handles.get_mut(&Self::key(handle)) {
            Some(Open::File(f)) => Ok(f),
            _ => Err(StatusCode::Failure),
        }
    }
}

fn attrs_of(path: &Path, follow: bool) -> Result<FileAttributes, StatusCode> {
    let meta = if follow { std::fs::metadata(path) } else { std::fs::symlink_metadata(path) };
    meta.map(|m| FileAttributes::from(&m)).map_err(code)
}

impl russh_sftp::server::Handler for Files {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(&mut self, _version: u32, _extensions: HashMap<String, String>) -> Result<Version, Self::Error> {
        Ok(Version::new())
    }

    async fn realpath(&mut self, id: u32, path: SftpString) -> Result<Name, Self::Error> {
        let mut parts: Vec<&str> = Vec::new();
        for p in path.split('/') {
            match p {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                p => parts.push(p),
            }
        }
        Ok(Name { id, files: vec![File::dummy(format!("/{}", parts.join("/")))] })
    }

    async fn opendir(&mut self, id: u32, path: SftpString) -> Result<Handle, Self::Error> {
        let mut files = Vec::new();
        for e in std::fs::read_dir(self.local(&path)).map_err(code)? {
            let e = e.map_err(code)?;
            files.push(File::new(e.file_name().to_string_lossy().into_owned(), attrs_of(&e.path(), false)?));
        }
        Ok(self.handle(id, Open::Dir(Some(files))))
    }

    async fn readdir(&mut self, id: u32, handle: SftpHandle) -> Result<Name, Self::Error> {
        match self.handles.get_mut(&Self::key(&handle)) {
            Some(Open::Dir(files)) => files.take().map(|files| Name { id, files }).ok_or(StatusCode::Eof),
            _ => Err(StatusCode::Failure),
        }
    }

    async fn open(&mut self, id: u32, filename: SftpString, pflags: OpenFlags, _attrs: FileAttributes) -> Result<Handle, Self::Error> {
        let f = std::fs::OpenOptions::from(pflags).open(self.local(&filename)).map_err(code)?;
        Ok(self.handle(id, Open::File(f)))
    }

    async fn close(&mut self, id: u32, handle: SftpHandle) -> Result<Status, Self::Error> {
        self.handles.remove(&Self::key(&handle));
        Ok(ok(id))
    }

    async fn read(&mut self, id: u32, handle: SftpHandle, offset: u64, len: u32) -> Result<Data, Self::Error> {
        let f = self.file(&handle)?;
        f.seek(SeekFrom::Start(offset)).map_err(code)?;
        let mut data = vec![0; len as usize];
        let n = f.read(&mut data).map_err(code)?;
        if n == 0 {
            return Err(StatusCode::Eof);
        }
        data.truncate(n);
        Ok(Data { id, data })
    }

    async fn write(&mut self, id: u32, handle: SftpHandle, offset: u64, data: Vec<u8>) -> Result<Status, Self::Error> {
        let f = self.file(&handle)?;
        f.seek(SeekFrom::Start(offset)).map_err(code)?;
        f.write_all(&data).map_err(code)?;
        Ok(ok(id))
    }

    async fn stat(&mut self, id: u32, path: SftpString) -> Result<Attrs, Self::Error> {
        Ok(Attrs { id, attrs: attrs_of(&self.local(&path), true)? })
    }

    async fn lstat(&mut self, id: u32, path: SftpString) -> Result<Attrs, Self::Error> {
        Ok(Attrs { id, attrs: attrs_of(&self.local(&path), false)? })
    }

    async fn fstat(&mut self, id: u32, handle: SftpHandle) -> Result<Attrs, Self::Error> {
        let meta = self.file(&handle)?.metadata().map_err(code)?;
        Ok(Attrs { id, attrs: FileAttributes::from(&meta) })
    }

    async fn setstat(&mut self, id: u32, path: SftpString, attrs: FileAttributes) -> Result<Status, Self::Error> {
        #[cfg(unix)]
        if let Some(mode) = attrs.permissions {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(self.local(&path), std::fs::Permissions::from_mode(mode)).map_err(code)?;
        }
        let _ = (&path, &attrs);
        Ok(ok(id))
    }

    async fn remove(&mut self, id: u32, filename: SftpString) -> Result<Status, Self::Error> {
        std::fs::remove_file(self.local(&filename)).map_err(code)?;
        Ok(ok(id))
    }

    async fn mkdir(&mut self, id: u32, path: SftpString, _attrs: FileAttributes) -> Result<Status, Self::Error> {
        std::fs::create_dir(self.local(&path)).map_err(code)?;
        Ok(ok(id))
    }

    async fn rmdir(&mut self, id: u32, path: SftpString) -> Result<Status, Self::Error> {
        std::fs::remove_dir(self.local(&path)).map_err(code)?;
        Ok(ok(id))
    }

    /// Like the plain protocol (no `posix-rename`): an existing target is an error.
    async fn rename(&mut self, id: u32, oldpath: SftpString, newpath: SftpString) -> Result<Status, Self::Error> {
        let to = self.local(&newpath);
        if to.exists() {
            return Err(StatusCode::Failure);
        }
        std::fs::rename(self.local(&oldpath), to).map_err(code)?;
        Ok(ok(id))
    }
}

/// Start a server whose SFTP root is `root`; returns (port, host public key).
pub async fn start_sftp(root: &Path, enabled: bool) -> (u16, PublicKey) {
    let key = host_key();
    let public = key.public_key().clone();
    let config = Arc::new(server::Config {
        keys: vec![key],
        methods: MethodSet::from(&[MethodKind::Password][..]),
        auth_rejection_time: Duration::from_millis(10),
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut srv = SftpServer { root: root.to_path_buf(), enabled, channels: HashMap::new() };
    tokio::spawn(async move {
        let _ = srv.run_on_socket(config, &listener).await;
    });
    (port, public)
}
