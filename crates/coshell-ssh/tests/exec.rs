//! Background commands (the AI assistant reads their output) over a real SSH session.

mod common;

use common::*;
use coshell_ssh::{HostKeyPolicy, connect_chain};

#[tokio::test(flavor = "multi_thread")]
async fn exec_collects_output_stderr_and_exit_status() {
    let (port, key) = start(Role::Target).await;
    let dir = tempfile::tempdir().unwrap();
    let kh = dir.path().join("known_hosts");
    trust(&kh, port, &key);
    let conn = connect_chain(&[hop(TARGET_USER, port)], &opts(&kh, HostKeyPolicy::Strict)).await.unwrap();

    let out = tokio::time::timeout(IO_TIMEOUT, conn.exec("ls -al", 1024)).await.expect("exec timed out").unwrap();
    assert_eq!(String::from_utf8_lossy(&out.output), "ran: ls -al\nwarning on stderr\n");
    assert_eq!((out.exit, out.truncated), (Some(7), false));

    let out = tokio::time::timeout(IO_TIMEOUT, conn.exec("ls -al", 5)).await.expect("exec timed out").unwrap();
    assert_eq!((out.output.as_slice(), out.truncated), (&b"ran: "[..], true), "kept to the limit");
}
