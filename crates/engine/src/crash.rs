//! Crash reports. A panic hook writes what happened (version, system, thread, message, place and
//! a backtrace) to a file in the crash folder, so a crash can be diagnosed afterwards, and a
//! marker file per running session catches the crashes no hook sees (a segfault, an abort, the
//! process being killed): a marker left behind by a process that is gone is reported at the next
//! start as an unclean exit. The newest [`KEEP`] reports are kept.
//!
//! Unclean-exit detection needs to ask whether a process is alive, which only Linux answers
//! without `unsafe`; elsewhere markers are written and removed but never reported.

use std::backtrace::Backtrace;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Reports kept in the crash folder (the oldest go first).
pub const KEEP: usize = 20;

/// A report is being written (a panic while writing one must not write another).
static IN_HOOK: AtomicBool = AtomicBool::new(false);
/// Reports written by this process (keeps file names apart within one second).
static COUNT: AtomicUsize = AtomicUsize::new(0);

/// Written into a session marker once the panic hook has reported for it.
const REPORTED: &str = "panic-reported";

/// The crash folder, next to the recovery folder. `SOLVECRAFT_CRASH_DIR` overrides it.
pub fn default_dir() -> Option<PathBuf> {
    // Tests never touch the user's real crash folder.
    if cfg!(test) {
        return Some(std::env::temp_dir().join(format!("solvecraft-test-crashes-{}", std::process::id())));
    }
    if let Some(d) = std::env::var_os("SOLVECRAFT_CRASH_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    crate::recovery::default_dir().and_then(|d| d.parent().map(|p| p.join("crashes")))
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// `secs` since the Unix epoch as `YYYY-MM-DD HH:MM:SS UTC`.
pub fn utc(secs: u64) -> String {
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    // Days to a calendar date (proleptic Gregorian).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// The text of a report.
pub fn format_report(version: &str, what: &str, thread: &str, message: &str, location: &str, backtrace: &str) -> String {
    format!(
        "SolveCraft crash report\nversion: {version}\ntime: {}\nsystem: {} {}\nkind: {what}\nthread: {thread}\nmessage: {message}\nlocation: {location}\n\nbacktrace:\n{backtrace}\n",
        utc(now_secs()),
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
}

/// Reports in `dir`, oldest first.
pub fn reports(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("crash-") && n.ends_with(".log")))
        .collect();
    v.sort();
    v
}

/// The newest report in `dir`.
pub fn latest(dir: &Path) -> Option<PathBuf> {
    reports(dir).pop()
}

/// Save `text` as a new report in `dir` (made if missing) and drop the oldest beyond [`KEEP`].
pub fn write_report(dir: &Path, text: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let n = COUNT.fetch_add(1, Ordering::SeqCst);
    let path = dir.join(format!("crash-{:012}-{}-{n:03}.log", now_secs(), std::process::id()));
    let mut f = std::fs::File::create(&path)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    let all = reports(dir);
    for old in all.iter().take(all.len().saturating_sub(KEEP)) {
        let _ = std::fs::remove_file(old);
    }
    Ok(path)
}

fn marker(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("open-{pid}"))
}

/// Install the panic hook: every panic (caught or not) is written to `dir` before the previous
/// hook runs. Call once, early.
pub fn install(dir: PathBuf, version: &'static str) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !IN_HOOK.swap(true, Ordering::SeqCst) {
            let thread = std::thread::current().name().unwrap_or("(unnamed)").to_string();
            let message = info.payload_as_str().unwrap_or("(not a text message)");
            let location = info.location().map_or_else(|| "(unknown)".to_string(), |l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
            let text = format_report(version, "panic", &thread, message, &location, &Backtrace::force_capture().to_string());
            if let Ok(path) = write_report(&dir, &text) {
                eprintln!("SolveCraft hit an internal error; report saved to {}", path.display());
                // The session's marker now says a report exists (an unclean exit needs no second one).
                let _ = std::fs::write(marker(&dir, std::process::id()), format!("{version}\n{REPORTED}\n"));
            }
            IN_HOOK.store(false, Ordering::SeqCst);
        }
        previous(info);
    }));
}

/// Is the process `pid` running? (Unknown elsewhere than Linux: assumed so.)
fn alive(pid: u32) -> bool {
    if cfg!(target_os = "linux") { Path::new("/proc").join(pid.to_string()).exists() } else { true }
}

/// Start a session: report the sessions that ended without a clean exit, then leave a marker
/// for this one (removed by [`end_session`]). Returns the reports written since the last start
/// (panics and unclean exits), oldest first.
pub fn begin_session(dir: &Path, version: &str) -> Vec<PathBuf> {
    let _ = std::fs::create_dir_all(dir);
    let me = std::process::id();
    let stamp = dir.join(".last_start");
    let since = std::fs::metadata(&stamp).and_then(|m| m.modified()).ok();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let Some(pid) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_prefix("open-")).and_then(|n| n.parse::<u32>().ok()) else {
            continue;
        };
        if pid == me || alive(pid) {
            continue;
        }
        let info = std::fs::read_to_string(&path).unwrap_or_default();
        if !info.contains(REPORTED) {
            let started =
                std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
            let was = info.lines().next().unwrap_or("?");
            let msg = format!(
                "process {pid} (version {was}, started {}) ended without a clean exit and left no panic report: a crash the app could not catch (an abort, a segfault or the process being killed)",
                utc(started)
            );
            let text = format_report(version, "unclean exit", "(unknown)", &msg, "(unknown)", "(none: nothing could be recorded)");
            let _ = write_report(dir, &text);
        }
        let _ = std::fs::remove_file(&path);
    }
    let fresh: Vec<PathBuf> = reports(dir)
        .into_iter()
        .filter(|p| match (since, std::fs::metadata(p).and_then(|m| m.modified()).ok()) {
            (Some(s), Some(m)) => m > s,
            (None, Some(_)) => true,
            _ => false,
        })
        .collect();
    let _ = std::fs::write(marker(dir, me), format!("{version}\n"));
    let _ = std::fs::write(&stamp, now_secs().to_string());
    fresh
}

/// End this session cleanly (removes its marker).
pub fn end_session(dir: &Path) {
    let _ = std::fs::remove_file(marker(dir, std::process::id()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("solvecraft-crash-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn dates_are_formatted_in_utc() {
        assert_eq!(utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(utc(1_000_000_000), "2001-09-09 01:46:40 UTC");
        assert_eq!(utc(1_791_656_982), "2026-10-10 18:29:42 UTC");
        assert_eq!(utc(951_782_400), "2000-02-29 00:00:00 UTC");
    }

    #[test]
    fn a_report_names_what_happened() {
        let t = format_report("1.2.3", "panic", "main", "boom", "src/x.rs:1:2", "frame 0");
        for want in ["version: 1.2.3", "kind: panic", "thread: main", "message: boom", "location: src/x.rs:1:2", "frame 0", "system: "] {
            assert!(t.contains(want), "missing `{want}` in {t}");
        }
    }

    #[test]
    fn only_the_newest_reports_are_kept() {
        let d = fresh_dir("keep");
        let mut last = PathBuf::new();
        for i in 0..KEEP + 7 {
            last = write_report(&d, &format!("report {i}")).unwrap();
        }
        let all = reports(&d);
        assert_eq!(all.len(), KEEP);
        assert_eq!(latest(&d), Some(last.clone()));
        assert_eq!(std::fs::read_to_string(last).unwrap(), format!("report {}", KEEP + 6));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_missing_folder_is_made() {
        let d = fresh_dir("make").join("deeper");
        assert!(write_report(&d, "x").is_ok());
        let _ = std::fs::remove_dir_all(d.parent().unwrap());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_dead_sessions_marker_is_an_unclean_exit() {
        let d = fresh_dir("unclean");
        std::fs::create_dir_all(&d).unwrap();
        // A process that is not running, one that is, and one that already has a panic report.
        let dead = 4_194_000;
        assert!(!alive(dead), "pid {dead} should not exist");
        std::fs::write(marker(&d, dead), "0.9.9\n").unwrap();
        std::fs::write(marker(&d, dead + 1), format!("0.9.9\n{REPORTED}\n")).unwrap();
        let parent = std::os::unix::process::parent_id();
        std::fs::write(marker(&d, parent), "0.9.9\n").unwrap();
        let fresh = begin_session(&d, "1.0.0");
        assert_eq!(fresh.len(), 1, "{fresh:?}");
        let text = std::fs::read_to_string(&fresh[0]).unwrap();
        assert!(text.contains("kind: unclean exit") && text.contains("process 4194000") && text.contains("0.9.9"), "{text}");
        assert!(!marker(&d, dead).exists() && !marker(&d, dead + 1).exists(), "dead markers are cleared");
        assert!(marker(&d, parent).exists(), "a live session's marker stays");
        assert!(marker(&d, std::process::id()).exists(), "this session has a marker");
        // The next start has nothing new, and a clean exit leaves no marker.
        end_session(&d);
        assert!(!marker(&d, std::process::id()).exists());
        assert!(begin_session(&d, "1.0.0").is_empty());
        end_session(&d);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The child half of [`a_real_panic_is_written_to_the_crash_folder`]: does nothing unless the
    /// parent asks, then installs the hook and panics.
    #[test]
    fn child_panics() {
        let Some(dir) = std::env::var_os("SOLVECRAFT_CRASH_TEST_DIR") else { return };
        install(PathBuf::from(dir), "9.9.9");
        panic!("crash log test panic");
    }

    #[test]
    fn a_real_panic_is_written_to_the_crash_folder() {
        let d = fresh_dir("panic");
        let exe = std::env::current_exe().unwrap();
        let out = std::process::Command::new(exe)
            .args(["--exact", "crash::tests::child_panics", "--test-threads=1"])
            .env("SOLVECRAFT_CRASH_TEST_DIR", &d)
            .output()
            .unwrap();
        assert!(!out.status.success(), "the child should have failed with its panic");
        let r = latest(&d).expect("the panic left a report");
        let text = std::fs::read_to_string(r).unwrap();
        for want in ["version: 9.9.9", "kind: panic", "message: crash log test panic", "crash.rs:", "backtrace:"] {
            assert!(text.contains(want), "missing `{want}` in {text}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
