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

#[cfg(target_os = "macos")]
fn local_now() -> Result<Stats, String> {
    let mut s = Stats::default();
    let ncpu: f64 = sh("sysctl -n hw.ncpu").trim().parse().unwrap_or(1.0);
    let cpu: f64 = sh("ps -A -o %cpu").lines().skip(1).filter_map(|l| l.trim().parse::<f64>().ok()).sum();
    s.cpu = pct(cpu, ncpu * 100.0);
    let total: f64 = sh("sysctl -n hw.memsize").trim().parse().unwrap_or(0.0);
    let vm = sh("vm_stat");
    let page: f64 = vm.split("page size of ").nth(1).and_then(|r| r.split_whitespace().next()).and_then(|n| n.parse().ok()).unwrap_or(16384.0);
    let pages = |key: &str| {
        vm.lines().find(|l| l.starts_with(key)).and_then(|l| l.rsplit(':').next()).and_then(|n| n.trim().trim_end_matches('.').parse::<f64>().ok()).unwrap_or(0.0)
    };
    let used = (pages("Pages active") + pages("Pages wired down") + pages("Pages occupied by compressor")) * page;
    s.mem = pct(used, total);
    disk_line(&sh("df -P /System/Volumes/Data 2>/dev/null || df -P /"), &mut s);
    if s.disk_mount == "/System/Volumes/Data" {
        s.disk_mount = "/".into();
    }
    let name = sh("sw_vers -productName").trim().to_string();
    let ver = sh("sw_vers -productVersion").trim().to_string();
    s.os = (!name.is_empty()).then(|| format!("{name} {ver}"));
    Ok(s)
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
}
