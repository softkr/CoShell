//! File listing, transfers and management over a real SSH session's SFTP subsystem.

mod common;

use common::sftp_server::start_sftp;
use common::*;
use coshell_ssh::{Connection, FileKind, HostKeyPolicy, SshError, TransferProgress, connect_chain};

async fn connect(root: &std::path::Path, enabled: bool) -> (Connection, tempfile::TempDir) {
    let (port, key) = start_sftp(root, enabled).await;
    let dir = tempfile::tempdir().unwrap();
    let kh = dir.path().join("known_hosts");
    trust(&kh, port, &key);
    (connect_chain(&[hop(TARGET_USER, port)], &opts(&kh, HostKeyPolicy::Strict)).await.unwrap(), dir)
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_folders_first_and_follows_links() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("logs")).unwrap();
    std::fs::write(root.path().join("app.conf"), b"port=80\n").unwrap();
    std::fs::write(root.path().join("Zeta.txt"), b"z").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.path().join("logs"), root.path().join("current")).unwrap();
    let (conn, _kh) = connect(root.path(), true).await;
    let sftp = tokio::time::timeout(IO_TIMEOUT, conn.sftp()).await.expect("sftp timed out").unwrap();

    assert_eq!(sftp.resolve(".").await.unwrap(), "/");
    let list = sftp.list("/").await.unwrap();
    let names: Vec<_> = list.iter().map(|e| (e.name.as_str(), e.kind, e.link)).collect();
    #[cfg(unix)]
    assert_eq!(
        names,
        [("current", FileKind::Dir, true), ("logs", FileKind::Dir, false), ("app.conf", FileKind::File, false), ("Zeta.txt", FileKind::File, false)]
    );
    let conf = list.iter().find(|e| e.name == "app.conf").unwrap();
    assert_eq!(conf.size, 8);
    assert!(conf.modified.is_some());
    assert!(matches!(sftp.list("/missing").await, Err(SshError::Sftp(_))));
}

#[tokio::test(flavor = "multi_thread")]
async fn transfers_report_progress_and_leave_no_partial_files() {
    let root = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    // Bigger than one chunk, so progress moves in steps.
    let body: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(local.path().join("up.bin"), &body).unwrap();
    let (conn, _kh) = connect(root.path(), true).await;
    let sftp = conn.sftp().await.unwrap();

    let p = TransferProgress::default();
    sftp.upload(&local.path().join("up.bin"), "/up.bin", &p).await.unwrap();
    assert_eq!((p.done(), p.total()), (body.len() as u64, body.len() as u64));
    assert_eq!(std::fs::read(root.path().join("up.bin")).unwrap(), body);

    let p = TransferProgress::default();
    sftp.download("/up.bin", &local.path().join("down.bin"), &p).await.unwrap();
    assert_eq!(p.done(), body.len() as u64);
    assert_eq!(std::fs::read(local.path().join("down.bin")).unwrap(), body);

    // Cancelled before it starts: nothing appears on either side.
    let p = TransferProgress::default();
    p.cancel();
    assert!(matches!(sftp.download("/up.bin", &local.path().join("never.bin"), &p).await, Err(SshError::Cancelled(_))));
    assert!(matches!(sftp.upload(&local.path().join("up.bin"), "/never.bin", &p).await, Err(SshError::Cancelled(_))));
    let leftovers = |d: &std::path::Path| std::fs::read_dir(d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|n| n.contains("never")).count();
    assert_eq!((leftovers(local.path()), leftovers(root.path())), (0, 0));
}

#[tokio::test(flavor = "multi_thread")]
async fn upload_replaces_a_file_and_keeps_its_permissions() {
    let root = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("run.sh"), b"old").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root.path().join("run.sh"), std::fs::Permissions::from_mode(0o750)).unwrap();
    }
    std::fs::write(local.path().join("run.sh"), b"new content").unwrap();
    let (conn, _kh) = connect(root.path(), true).await;
    let sftp = conn.sftp().await.unwrap();

    sftp.upload(&local.path().join("run.sh"), "/run.sh", &TransferProgress::default()).await.unwrap();
    assert_eq!(std::fs::read(root.path().join("run.sh")).unwrap(), b"new content");
    #[cfg(unix)]
    assert_eq!(sftp.list("/").await.unwrap()[0].permissions, Some(0o750));
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1, "no temporary file left");
}

#[tokio::test(flavor = "multi_thread")]
async fn manages_files_and_removes_folders_without_following_links() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("keep.txt"), b"keep").unwrap();
    let (conn, _kh) = connect(root.path(), true).await;
    let sftp = conn.sftp().await.unwrap();

    sftp.mkdir("/site").await.unwrap();
    sftp.mkdir("/site/assets").await.unwrap();
    std::fs::write(root.path().join("site/index.html"), b"<p>").unwrap();
    std::fs::write(root.path().join("site/assets/a.css"), b"a{}").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), root.path().join("site/assets/out")).unwrap();

    sftp.rename("/site/index.html", "/site/home.html").await.unwrap();
    assert!(root.path().join("site/home.html").exists());
    #[cfg(unix)]
    {
        sftp.chmod("/site/home.html", 0o600).await.unwrap();
        let e = sftp.list("/site").await.unwrap().into_iter().find(|e| e.name == "home.html").unwrap();
        assert_eq!(e.permissions, Some(0o600));
    }

    sftp.remove("/site/home.html").await.unwrap();
    sftp.remove("/site").await.unwrap();
    assert!(!root.path().join("site").exists());
    assert!(outside.path().join("keep.txt").exists(), "the link's target is untouched");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_without_sftp_says_so() {
    let root = tempfile::tempdir().unwrap();
    let (conn, _kh) = connect(root.path(), false).await;
    let r = tokio::time::timeout(IO_TIMEOUT, conn.sftp()).await.expect("sftp timed out");
    assert!(r.is_err());
}
