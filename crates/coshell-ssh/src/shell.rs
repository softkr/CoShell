//! Interactive PTY shell on the target, exposed as a `coshell_term::Transport`.

use std::io::Read;
use std::sync::mpsc;

use coshell_term::{GridSize, Transport};
use russh::ChannelMsg;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use crate::{Connection, SshError};

const TERM: &str = "xterm-256color";

enum Cmd {
    Data(Vec<u8>),
    Resize(GridSize),
}

/// Write side handed to the terminal session. Cheap, non-blocking.
pub struct SshTransport {
    tx: UnboundedSender<Cmd>,
}

impl Transport for SshTransport {
    fn write(&self, bytes: &[u8]) {
        let _ = self.tx.send(Cmd::Data(bytes.to_vec()));
    }
    fn resize(&self, size: GridSize) {
        let _ = self.tx.send(Cmd::Resize(size));
    }
}

/// Blocking reader fed by the async channel task; EOF when the channel closes.
struct ChannelReader {
    rx: mpsc::Receiver<Vec<u8>>,
    pending: Vec<u8>,
    pos: usize,
}

impl Read for ChannelReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.pending.len() {
            match self.rx.recv() {
                Ok(chunk) => {
                    self.pending = chunk;
                    self.pos = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.pending.len() - self.pos);
        out[..n].copy_from_slice(&self.pending[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// Open a PTY shell on the connection's target. Must run inside a tokio runtime.
/// When the shell ends (or the transport is dropped) the connection is closed.
pub async fn open_shell(
    conn: &Connection,
    size: GridSize,
) -> Result<(SshTransport, Box<dyn Read + Send>), SshError> {
    let label = conn.labels().last().cloned().unwrap_or_default();
    let ssh = |source| SshError::Ssh { hop: label.clone(), source };
    let target = conn.target();

    let channel = target.channel_open_session().await.map_err(ssh)?;
    channel
        .request_pty(
            false,
            TERM,
            u32::from(size.cols),
            u32::from(size.rows),
            u32::from(size.cols * size.cell_width),
            u32::from(size.rows * size.cell_height),
            &[],
        )
        .await
        .map_err(ssh)?;
    channel.request_shell(true).await.map_err(ssh)?;

    let (mut read, write) = channel.split();
    let (cmd_tx, mut cmd_rx) = unbounded_channel::<Cmd>();
    let (data_tx, data_rx) = mpsc::channel::<Vec<u8>>();

    let conn = conn.clone(); // keep every hop open for the session's lifetime
    tokio::spawn(async move {
        loop {
            tokio::select! {
                msg = read.wait() => match msg {
                    Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                        if data_tx.send(data.to_vec()).is_err() { break; }
                    }
                    Some(ChannelMsg::Failure) => {
                        let _ = data_tx.send(b"\r\n[CoShell] remote shell request was refused\r\n".to_vec());
                        break;
                    }
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => break,
                    Some(_) => {}
                },
                cmd = cmd_rx.recv() => match cmd {
                    Some(Cmd::Data(bytes)) => {
                        if write.data_bytes(bytes).await.is_err() { break; }
                    }
                    Some(Cmd::Resize(s)) => {
                        let _ = write
                            .window_change(
                                u32::from(s.cols),
                                u32::from(s.rows),
                                u32::from(s.cols * s.cell_width),
                                u32::from(s.rows * s.cell_height),
                            )
                            .await;
                    }
                    // Transport dropped (tab closed).
                    None => break,
                },
            }
        }
        let _ = write.close().await;
        conn.disconnect().await;
        // data_tx dropped here → reader sees EOF → TermEvent::Exited.
    });

    Ok((SshTransport { tx: cmd_tx }, Box::new(ChannelReader { rx: data_rx, pending: Vec::new(), pos: 0 })))
}
