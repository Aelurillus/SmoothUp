use std::{
    ffi::OsString,
    io::Write,
    path::PathBuf,
    sync::{atomic::{AtomicBool, Ordering}, Arc},
    thread,
    time::{Duration, Instant},
};

use smoothup::process;

fn fixture_args(name: &str) -> Vec<OsString> {
    ["--exact", name, "--ignored", "--nocapture"].into_iter().map(OsString::from).collect()
}

#[test]
fn capture_limit_terminates_a_child_that_keeps_running() {
    let cancel = Arc::new(AtomicBool::new(false));
    let started = Instant::now();
    let error = process::capture(&std::env::current_exe().unwrap(), &fixture_args("fixture_overflow"), &cancel, 4096).unwrap_err();
    assert!(error.contains("上限"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(4), "child was not terminated promptly");
}

#[test]
fn cancellation_terminates_a_child_with_open_pipes() {
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&cancel);
    let trigger = thread::spawn(move || {
        thread::sleep(Duration::from_millis(150));
        signal.store(true, Ordering::SeqCst);
    });
    let started = Instant::now();
    let error = process::capture(&std::env::current_exe().unwrap(), &fixture_args("fixture_wait"), &cancel, 4096).unwrap_err();
    trigger.join().unwrap();
    assert!(error.contains("取消"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(4), "cancelled child was not terminated promptly");
}

struct TempDir(PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

#[test]
fn executable_paths_with_shell_characters_are_literal() {
    let root = TempDir(std::env::temp_dir().join(format!("smoothup-literal-{}", std::process::id())));
    std::fs::create_dir(&root.0).unwrap();
    let path = root.0.join("路径 空格 & ' [x] ;.exe");
    std::fs::hard_link(std::env::current_exe().unwrap(), &path).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let output = process::capture(&path, &fixture_args("fixture_literal"), &cancel, 4096).unwrap();
    assert!(String::from_utf8(output).unwrap().contains("literal-path-executed"));
}

#[test]
#[ignore = "controlled child process, launched by capture_limit test"]
fn fixture_overflow() {
    std::io::stdout().write_all(&[b'x'; 16384]).unwrap();
    std::io::stdout().flush().unwrap();
    thread::sleep(Duration::from_secs(8));
}

#[test]
#[ignore = "controlled child process, launched by cancellation test"]
fn fixture_wait() { thread::sleep(Duration::from_secs(8)); }

#[test]
#[ignore = "controlled child process, launched by literal-path test"]
fn fixture_literal() { println!("literal-path-executed"); }
