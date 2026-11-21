//! Live resource figures for the sidebar card and the status bar: CPU, memory,
//! disk use, the OS name and the round-trip time. Read-only and out of band: SSH
//! sessions use a separate exec channel, local shells run a few system tools.

use std::time::{Duration, Instant};

/// How often the active session is polled.
pub const POLL_EVERY: Duration = Duration::from_secs(5);
/// A poll that takes longer than this is dropped.
const POLL_TIMEOUT: Duration = Duration::from_secs(8);
const OUTPUT_LIMIT: usize = 16 * 1024;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub cpu: u8,
    pub mem: u8,
    pub disk: u8,
    /// Mount point the disk figure is for (`/var`, `/`).
    pub disk_mount: String,
    pub os: Option<String>,
    /// Round trip of a no-op command (SSH only).
    pub latency_ms: Option<u64>,
}

/// Two CPU samples half a second apart, memory, the disk holding /var, and the OS name (Linux).
const LINUX_SCRIPT: &str = "head -n1 /proc/stat; sleep 0.5; head -n1 /proc/stat; \
grep -E '^(MemTotal|MemAvailable):' /proc/meminfo; df -P /var 2>/dev/null | tail -n1; \
. /etc/os-release 2>/dev/null && echo \"OS=$PRETTY_NAME\"";

fn pct(used: f64, total: f64) -> u8 {
    if total <= 0.0 { 0 } else { (used / total * 100.0).round().clamp(0.0, 100.0) as u8 }
}

/// Output of [`LINUX_SCRIPT`].
pub fn parse_linux(out: &str) -> Option<Stats> {
    let cpu_lines: Vec<Vec<u64>> = out
        .lines()
        .filter(|l| l.starts_with("cpu "))
        .map(|l| l.split_whitespace().skip(1).filter_map(|n| n.parse().ok()).collect())
        .collect();
    let mut s = Stats::default();
    if let [a, b] = cpu_lines.as_slice() {
        // idle + iowait count as idle
        let idle = |v: &[u64]| v.get(3).copied().unwrap_or(0) + v.get(4).copied().unwrap_or(0);
        let total = |v: &[u64]| v.iter().sum::<u64>();
        let dt = total(b).saturating_sub(total(a)) as f64;
        let di = idle(b).saturating_sub(idle(a)) as f64;
        s.cpu = pct(dt - di, dt);
    }
    let kb = |key: &str| {
        out.lines().find(|l| l.starts_with(key)).and_then(|l| l.split_whitespace().nth(1)).and_then(|n| n.parse::<f64>().ok())
    };
    let (total, avail) = (kb("MemTotal:")?, kb("MemAvailable:")?);
    s.mem = pct(total - avail, total);
    disk_line(out, &mut s);
    s.os = out.lines().find_map(|l| l.strip_prefix("OS=")).map(|o| o.trim().to_string()).filter(|o| !o.is_empty());
    Some(s)
}

/// The `df -P` data line: `fs blocks used avail capacity% mount`.
fn disk_line(out: &str, s: &mut Stats) {
    if let Some(cols) = out.lines().map(|l| l.split_whitespace().collect::<Vec<_>>()).find(|c| c.len() >= 6 && c[4].ends_with('%')) {
        s.disk = cols[4].trim_end_matches('%').parse().unwrap_or(0);
        s.disk_mount = cols[5..].join(" ");
    }
}

/// Poll a server over its SSH connection.
pub async fn remote(conn: coshell_ssh::Connection) -> Result<Stats, String> {
    let run = async {
        let start = Instant::now();
        conn.exec(":", 64).await.map_err(|e| e.to_string())?;
        let rtt = start.elapsed();
        let out = conn.exec(LINUX_SCRIPT, OUTPUT_LIMIT).await.map_err(|e| e.to_string())?;
        let mut s = parse_linux(&String::from_utf8_lossy(&out.output)).ok_or_else(|| "unsupported system".to_string())?;
        // Opening a channel and running the command are two round trips.
        s.latency_ms = Some((rtt.as_millis() / 2) as u64);
        Ok(s)
    };
    tokio::time::timeout(POLL_TIMEOUT, run).await.map_err(|_| "timeout".to_string())?
}

/// Poll this computer.
pub async fn local() -> Result<Stats, String> {
    tokio::task::spawn_blocking(local_now).await.map_err(|e| e.to_string())?
}

fn sh(script: &str) -> String {
    std::process::Command::new("sh")
        .args(["-c", script])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Activity Monitor's figures. CPU: user + system over a one-second sample (`top`), not
/// the decaying per-process average `ps` reports. Memory: its "Memory Used", which is the
/// physical memory minus free pages and file-backed pages (cached files) — not the sum of
/// app, wired and compressed memory it lists below. Disk: Finder's and System Settings'
/// view of the startup disk, the whole APFS container less the space available for
/// important use, rather than `df` on the data volume alone (which leaves out the system
/// volume, VM and snapshots).
#[cfg(target_os = "macos")]
fn local_now() -> Result<Stats, String> {
    let total: f64 = sh("sysctl -n hw.memsize").trim().parse().unwrap_or(0.0);
    let mut s = Stats {
        cpu: parse_top_cpu(&sh("top -l 2 -n 0 -s 1")).unwrap_or(0),
        mem: parse_vm_stat(&sh("vm_stat"), total).unwrap_or(0),
        ..Default::default()
    };
    match volume_used("/") {
        Some(p) => s.disk = p,
        None => disk_line(&sh("df -P /System/Volumes/Data 2>/dev/null || df -P /"), &mut s),
    }
    s.disk_mount = "/".into();
    let name = sh("sw_vers -productName").trim().to_string();
    let ver = sh("sw_vers -productVersion").trim().to_string();
    s.os = (!name.is_empty()).then(|| format!("{name} {ver}"));
    Ok(s)
}

/// User + system CPU from the last `CPU usage:` line of `top -l 2` (the first sample covers
/// the time since boot).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_top_cpu(out: &str) -> Option<u8> {
    let line = out.lines().rfind(|l| l.starts_with("CPU usage:"))?;
    let field = |name: &str| {
        line.split(',').find(|f| f.trim_end().ends_with(name)).and_then(|f| f.split_whitespace().rev().nth(1)).and_then(|n| n.trim_end_matches('%').parse::<f64>().ok())
    };
    Some(pct(field("user")? + field("sys")?, 100.0))
}

/// Activity Monitor's "Memory Used" from `vm_stat`: physical memory less free and
/// file-backed pages.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_vm_stat(vm: &str, total: f64) -> Option<u8> {
    let page: f64 = vm.split("page size of ").nth(1)?.split_whitespace().next()?.parse().ok()?;
    let pages = |key: &str| vm.lines().find(|l| l.starts_with(key)).and_then(|l| l.rsplit(':').next()).and_then(|n| n.trim().trim_end_matches('.').parse::<f64>().ok());
    let free = (pages("Pages free")? + pages("File-backed pages")?) * page;
    Some(pct((total - free).max(0.0), total))
}

/// Share of the volume at `path` in use, counting purgeable space as free, as Finder does.
#[cfg(target_os = "macos")]
fn volume_used(path: &str) -> Option<u8> {
    use objc2_foundation::{NSNumber, NSString, NSURL, NSURLResourceKey, NSURLVolumeAvailableCapacityForImportantUsageKey, NSURLVolumeTotalCapacityKey};
    objc2::rc::autoreleasepool(|_| {
        let url = NSURL::fileURLWithPath(&NSString::from_str(path));
        let get = |key: &NSURLResourceKey| -> Option<f64> {
            let mut v = None;
            // SAFETY: both keys hold an NSNumber, and `v` is checked before use.
            unsafe { url.getResourceValue_forKey_error(&mut v, key) }.ok()?;
            Some(v?.downcast::<NSNumber>().ok()?.as_f64())
        };
        // SAFETY: Foundation's constant keys.
        let (total, free) = unsafe { (get(NSURLVolumeTotalCapacityKey)?, get(NSURLVolumeAvailableCapacityForImportantUsageKey)?) };
        (total > 0.0).then(|| pct(total - free, total))
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn local_now() -> Result<Stats, String> {
    parse_linux(&sh(LINUX_SCRIPT)).ok_or_else(|| "unsupported system".to_string())
}

#[cfg(windows)]
fn local_now() -> Result<Stats, String> {
    Err("unsupported system".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_output_parses() {
        let out = "cpu  100 0 100 700 100 0 0 0 0 0\n\
cpu  150 0 150 750 100 0 0 0 0 0\n\
MemTotal:       16000000 kB\n\
MemAvailable:    6000000 kB\n\
/dev/sda2         83886080 76546048 7340032      91% /var\n\
OS=Ubuntu 22.04.4 LTS\n";
        let s = parse_linux(out).unwrap();
        assert_eq!(s.cpu, 67, "100 busy of 150 ticks");
        assert_eq!(s.mem, 63);
        assert_eq!((s.disk, s.disk_mount.as_str()), (91, "/var"));
        assert_eq!(s.os.as_deref(), Some("Ubuntu 22.04.4 LTS"));
        assert!(parse_linux("garbage").is_none());
    }

    #[test]
    fn macos_cpu_is_the_second_top_sample() {
        let out = "Processes: 700 total\nCPU usage: 12.50% user, 8.00% sys, 79.50% idle \n\
Processes: 701 total\nCPU usage: 3.65% user, 2.46% sys, 93.87% idle \n";
        assert_eq!(parse_top_cpu(out), Some(6));
        assert_eq!(parse_top_cpu("nothing"), None);
    }

    #[test]
    fn macos_memory_matches_activity_monitor() {
        // Sampled together with Activity Monitor showing 26.09 GB used of 64 GB.
        let vm = "Mach Virtual Memory Statistics: (page size of 16384 bytes)\n\
Pages free:                                   440436.\n\
Pages active:                                1715021.\n\
Pages inactive:                              1704015.\n\
Pages speculative:                             15075.\n\
Pages wired down:                             259272.\n\
Pages purgeable:                              112496.\n\
File-backed pages:                           2044527.\n\
Anonymous pages:                             1389584.\n\
Pages occupied by compressor:                   8708.\n";
        let total = 64.0 * 1024.0 * 1024.0 * 1024.0;
        assert_eq!(parse_vm_stat(vm, total), Some(pct(26.09, 64.0)));
        assert_eq!(parse_vm_stat("garbage", total), None);
    }
}
