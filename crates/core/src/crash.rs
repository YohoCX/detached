//! Crash reports: the panic hook writes one to `<config dir>/crashes/<UTC time>.txt`.

use std::backtrace::Backtrace;
use std::panic::{Location, PanicHookInfo};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs, io, thread};

const KEEP: usize = 20;

static SAVED: AtomicBool = AtomicBool::new(false);

/// Writes the report of the panic `info`, keeping the newest 20. It never panics and loses the
/// report on an IO error.
pub fn save(config_dir: &Path, info: &PanicHookInfo) {
    let payload = info.payload();
    let message = (payload.downcast_ref::<&str>().copied())
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str));
    let message = message.unwrap_or("(no message)");
    // A panic unwinding into GPUI's `extern "C"` frames panics again there, with nothing to tell.
    if message.starts_with("panic in a function that cannot unwind")
        && SAVED.load(Ordering::Relaxed)
    {
        return;
    }
    SAVED.store(true, Ordering::Relaxed);
    let _ = write(
        &config_dir.join("crashes"),
        &report(message, info.location()),
    );
}

/// The newest report, by its name.
pub fn newest(config_dir: &Path) -> Option<PathBuf> {
    reports(&config_dir.join("crashes")).ok()?.pop()
}

fn report(message: &str, location: Option<&Location>) -> String {
    let at = location.map_or("unknown".into(), |l| l.to_string());
    format!(
        "Detached {} on {} {}\nthread: {}\npanic: {message}\nat: {at}\n{}\n{:#}",
        env!("CARGO_PKG_VERSION"),
        env::consts::OS,
        env::consts::ARCH,
        thread::current().name().unwrap_or("unnamed"),
        image(),
        Backtrace::force_capture(),
    )
}

/// Where the executable is loaded, for `atos -l`.
#[cfg(target_os = "macos")]
fn image() -> String {
    // libc's declarations of these are deprecated for the mach2 crate.
    unsafe extern "C" {
        fn _dyld_get_image_header(image: u32) -> *const libc::c_void;
        fn _dyld_get_image_vmaddr_slide(image: u32) -> isize;
    }
    // SAFETY: image 0, the executable, is always loaded.
    let (header, slide) = unsafe { (_dyld_get_image_header(0), _dyld_get_image_vmaddr_slide(0)) };
    format!("load address: {header:p}, slide: {slide:#x}\n")
}

#[cfg(not(target_os = "macos"))]
fn image() -> String {
    String::new()
}

fn write(dir: &Path, text: &str) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let t = crate::os::utc(now.as_secs() as i64);
    let name = format!(
        "{:04}-{:02}-{:02}T{:02}-{:02}-{:02}.{:03}Z.txt",
        t.tm_year + 1900,
        t.tm_mon + 1,
        t.tm_mday,
        t.tm_hour,
        t.tm_min,
        t.tm_sec,
        now.subsec_millis()
    );
    fs::write(dir.join(name), text)?;
    for old in reports(dir)?.iter().rev().skip(KEEP) {
        let _ = fs::remove_file(old);
    }
    Ok(())
}

/// Oldest first: the names are UTC times.
fn reports(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut all: Vec<_> = fs::read_dir(dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "txt"))
        .collect();
    all.sort();
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_holds_the_message_its_place_and_the_backtrace() {
        let at = Location::caller();
        let text = report("boom", Some(at));
        assert!(text.contains("panic: boom\n"), "{text}");
        assert!(text.contains(&format!("at: {at}\n")), "{text}");
        assert!(text.contains(env!("CARGO_PKG_VERSION")), "{text}");
        assert!(
            text.contains("a_report_holds_the_message_its_place_and_the_backtrace"),
            "{text}"
        );
    }

    #[test]
    fn the_newest_20_reports_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let crashes = dir.path().join("crashes");
        fs::create_dir(&crashes).unwrap();
        for i in 0..25 {
            fs::write(
                crashes.join(format!("2020-01-01T00-00-{i:02}.000Z.txt")),
                "",
            )
            .unwrap();
        }
        write(&crashes, "new").unwrap();
        let left = reports(&crashes).unwrap();
        assert_eq!(left.len(), 20);
        assert!(left[0].ends_with("2020-01-01T00-00-06.000Z.txt"));
        let newest = newest(dir.path()).unwrap();
        assert_eq!(fs::read_to_string(newest).unwrap(), "new");
    }
}
