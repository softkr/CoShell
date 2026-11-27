//! Live resource figures for the sidebar card and the status bar: CPU, memory,
//! disk use, the OS name and the round-trip time. Read-only and out of band: SSH
//! sessions use a separate exec channel, local shells run a few system tools.

use std::time::{Duration, Instant};

/// How often this computer is asked for its figures (servers send theirs: see `watch`).
pub const POLL_EVERY: Duration = Duration::from_secs(2);
/// A poll that takes longer than this is dropped.
const POLL_TIMEOUT: Duration = Duration::from_secs(8);

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

/// How much time the resource card's graph shows (s).
pub const TREND_SPAN: f32 = 60.0;
/// How far behind now the graph's right edge is (s): longer than readings take to come, so
/// the next one is always there before the edge reaches it and the graph never waits. A
/// server sends one a second; this computer is asked every `POLL_EVERY` and takes a second
/// to answer.
pub fn trend_delay(local: bool) -> f32 {
    if local { 3.5 } else { 1.8 }
}
/// How many readings the graph keeps: what it shows, and a few either side of it.
pub const TREND_LEN: usize = 68;
/// How long the bars take to move to a new reading (s).
#[cfg_attr(test, allow(dead_code))]
pub const GLIDE: f32 = 0.6;

/// What the resource card remembers between readings: the last few CPU and memory figures,
/// and the reading before the latest with when the latest came, so the bars glide over.
#[derive(Debug, Default, Clone)]
pub struct Trend {
    pub cpu: std::collections::VecDeque<u8>,
    pub mem: std::collections::VecDeque<u8>,
    /// When each reading came.
    pub times: std::collections::VecDeque<std::time::Instant>,
    /// CPU, memory, disk before the latest reading.
    pub before: Option<(u8, u8, u8)>,
    pub at: Option<std::time::Instant>,
}

impl Trend {
    /// A new reading came in; `last` is the one it replaces.
    pub fn push(&mut self, last: Option<&Stats>, new: &Stats) {
        for (q, v) in [(&mut self.cpu, new.cpu), (&mut self.mem, new.mem)] {
            q.push_back(v);
            if q.len() > TREND_LEN {
                q.pop_front();
            }
        }
        let now = std::time::Instant::now();
        self.times.push_back(now);
        if self.times.len() > TREND_LEN {
            self.times.pop_front();
        }
        self.before = last.map(|s| (s.cpu, s.mem, s.disk));
        self.at = Some(now);
    }

    /// How long ago each reading came (s), oldest first. The graph places readings by their
    /// age, so it runs on at one steady pace whenever they happen to arrive.
    pub fn ages(&self) -> Vec<f32> {
        // In tests: as if they came exactly on time, the newest just reaching the edge.
        #[cfg(test)]
        return (0..self.times.len()).rev().map(|i| trend_delay(false) + i as f32).collect();
        #[cfg(not(test))]
        self.times.iter().map(|t| t.elapsed().as_secs_f32()).collect()
    }

    /// How far the bars have moved from the reading before to the latest (0 → 1).
    pub fn glide(&self) -> f32 {
        #[cfg(test)]
        return 1.0;
        #[cfg(not(test))]
        self.at.map_or(1.0, |at| (at.elapsed().as_secs_f32() / GLIDE).min(1.0))
    }
}

/// A reading a second, for as long as someone is listening (Linux): the OS name once, then
/// each second the CPU counters and memory (and every tenth second the disk holding /var),
/// closed by a line of `--`.
///
/// It stops by itself: when the channel closes its output has nowhere to go and the next
/// line it prints ends it, and it never runs longer than an hour in any case (the watcher
/// starts it again).
const WATCH_SCRIPT: &str = ". /etc/os-release 2>/dev/null; echo \"OS=$PRETTY_NAME\"; i=0; \
while [ $i -lt 3600 ]; do \
head -n1 /proc/stat || exit; \
grep -E '^(MemTotal|MemAvailable):' /proc/meminfo; \
[ $((i % 10)) -eq 0 ] && df -P /var 2>/dev/null | tail -n1; \
echo -- || exit; i=$((i+1)); sleep 1; done";
/// What a watch says when it ends after having given readings (so it is worth starting again).
pub const WATCH_ENDED: &str = "ended";
/// The round trip is timed once in this many readings.
const LATENCY_EVERY: u32 = 10;
/// A line of output longer than this is not ours: stop listening.
const LINE_LIMIT: usize = 64 * 1024;

fn pct(used: f64, total: f64) -> u8 {
    if total <= 0.0 { 0 } else { (used / total * 100.0).round().clamp(0.0, 100.0) as u8 }
}

/// Turns the watcher's output into readings: CPU is what was busy between one second's
/// counters and the next's, and what is only said now and then (the OS, the disk) is kept.
#[derive(Default)]
pub struct Sampler {
    /// The CPU counters a second ago.
    counters: Option<Vec<u64>>,
    disk: Option<(u8, String)>,
    os: Option<String>,
}

impl Sampler {
    /// One second's lines (what came before a `--`). `None` until there is a second to
    /// compare with, or when the lines are not what the script prints.
    pub fn feed(&mut self, block: &str) -> Option<Stats> {
        if let Some(os) = block.lines().find_map(|l| l.strip_prefix("OS=")).map(|o| o.trim().to_string()).filter(|o| !o.is_empty()) {
            self.os = Some(os);
        }
        let mut s = Stats::default();
        disk_line(block, &mut s);
        if !s.disk_mount.is_empty() {
            self.disk = Some((s.disk, s.disk_mount.clone()));
        }
        let kb = |key: &str| block.lines().find(|l| l.starts_with(key)).and_then(|l| l.split_whitespace().nth(1)).and_then(|n| n.parse::<f64>().ok());
        let (total, avail) = (kb("MemTotal:")?, kb("MemAvailable:")?);
        s.mem = pct(total - avail, total);
        let now: Vec<u64> = block.lines().find(|l| l.starts_with("cpu "))?.split_whitespace().skip(1).filter_map(|n| n.parse().ok()).collect();
        let before = self.counters.replace(now.clone())?;
        // idle + iowait count as idle
        let idle = |v: &[u64]| v.get(3).copied().unwrap_or(0) + v.get(4).copied().unwrap_or(0);
        let total = |v: &[u64]| v.iter().sum::<u64>();
        let dt = total(&now).saturating_sub(total(&before)) as f64;
        let di = idle(&now).saturating_sub(idle(&before)) as f64;
        s.cpu = pct(dt - di, dt);
        if let Some((disk, mount)) = &self.disk {
            (s.disk, s.disk_mount) = (*disk, mount.clone());
        }
        s.os = self.os.clone();
        Some(s)
    }
}

/// The `df -P` data line: `fs blocks used avail capacity% mount`.
fn disk_line(out: &str, s: &mut Stats) {
    if let Some(cols) = out.lines().map(|l| l.split_whitespace().collect::<Vec<_>>()).find(|c| c.len() >= 6 && c[4].ends_with('%')) {
        s.disk = cols[4].trim_end_matches('%').parse().unwrap_or(0);
        s.disk_mount = cols[5..].join(" ");
    }
}

/// Watch a server over its SSH connection: one channel kept open, a reading a second.
/// Ends with an `Err` when the channel closes (the connection went, or the hour was up).
pub fn watch(conn: coshell_ssh::Connection) -> impl iced::futures::Stream<Item = Result<Stats, String>> {
    iced::stream::channel(16, async move |mut out: iced::futures::channel::mpsc::Sender<Result<Stats, String>>| {
        use iced::futures::SinkExt;
        let mut stream = match conn.exec_stream(WATCH_SCRIPT).await {
            Ok(s) => s,
            Err(e) => {
                let _ = out.send(Err(e.to_string())).await;
                return;
            }
        };
        let mut sampler = Sampler::default();
        let (mut pending, mut block) = (String::new(), String::new());
        let (mut latency, mut readings) = (None, 0u32);
        'listen: while let Some(bytes) = stream.next().await {
            pending.push_str(&String::from_utf8_lossy(&bytes));
            if pending.len() > LINE_LIMIT || block.len() > LINE_LIMIT {
                break;
            }
            while let Some(end) = pending.find('\n') {
                let line: String = pending.drain(..=end).collect();
                if line.trim() != "--" {
                    block.push_str(&line);
                    continue;
                }
                let reading = sampler.feed(&block);
                block.clear();
                let Some(mut s) = reading else { continue };
                // Now and then, how long a round trip takes: opening a channel and running
                // a command that does nothing are two of them.
                if readings % LATENCY_EVERY == 0 {
                    let start = Instant::now();
                    if let Ok(Ok(_)) = tokio::time::timeout(POLL_TIMEOUT, conn.exec(":", 64)).await {
                        latency = Some((start.elapsed().as_millis() / 2) as u64);
                    }
                }
                readings = readings.wrapping_add(1);
                s.latency_ms = latency;
                if out.send(Ok(s)).await.is_err() {
                    break 'listen;
                }
            }
        }
        let _ = out.send(Err(if readings > 0 { WATCH_ENDED.into() } else { "nothing to read".to_string() })).await;
    })
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
    fn a_servers_readings_come_from_one_second_and_the_next() {
        let mut w = Sampler::default();
        let first = "OS=Ubuntu 22.04.4 LTS\ncpu  100 0 100 700 100 0 0 0 0 0\n\
MemTotal:       16000000 kB\nMemAvailable:    6000000 kB\n\
/dev/sda2         83886080 76546048 7340032      91% /var\n";
        assert_eq!(w.feed(first), None, "nothing to compare the counters with yet");
        // A second later: 100 of 150 ticks were busy. The disk and the OS are not said again.
        let s = w.feed("cpu  150 0 150 740 110 0 0 0 0 0\nMemTotal:       16000000 kB\nMemAvailable:    6000000 kB\n").unwrap();
        assert_eq!(s.cpu, 67);
        assert_eq!(s.mem, 63);
        assert_eq!((s.disk, s.disk_mount.as_str()), (91, "/var"), "kept from when it was last said");
        assert_eq!(s.os.as_deref(), Some("Ubuntu 22.04.4 LTS"));
        // An idle second.
        let s = w.feed("cpu  150 0 150 840 110 0 0 0 0 0\nMemTotal:       16000000 kB\nMemAvailable:    8000000 kB\n").unwrap();
        assert_eq!((s.cpu, s.mem), (0, 50));
        assert_eq!(w.feed("garbage"), None);
    }

    #[test]
    fn the_watcher_stops_by_itself() {
        // Bounded, and every turn of the loop prints (so a closed channel ends it).
        assert!(WATCH_SCRIPT.contains("-lt 3600") && WATCH_SCRIPT.contains("echo -- || exit") && WATCH_SCRIPT.contains("sleep 1"));
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
