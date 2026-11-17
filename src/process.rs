use std::{
    ffi::OsString,
    io::{BufReader, Read},
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const STDERR_LIMIT: usize = 32 * 1024;
const PROGRESS_LINE_LIMIT: usize = 4096;
const POLL_INTERVAL: Duration = Duration::from_millis(40);

#[derive(Default)]
struct Tail {
    bytes: Vec<u8>,
}

impl Tail {
    fn push(&mut self, chunk: &[u8]) {
        if chunk.len() >= STDERR_LIMIT {
            self.bytes.clear();
            self.bytes.extend_from_slice(&chunk[chunk.len() - STDERR_LIMIT..]);
            return;
        }
        let overflow = self.bytes.len().saturating_add(chunk.len()).saturating_sub(STDERR_LIMIT);
        if overflow > 0 {
            self.bytes.drain(..overflow);
        }
        self.bytes.extend_from_slice(chunk);
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).trim().to_owned()
    }
}

pub fn run(
    program: &Path,
    args: &[OsString],
    cancel: &Arc<AtomicBool>,
    verbose: bool,
    progress_duration: Option<f64>,
) -> Result<(), String> {
    if verbose {
        eprintln!("执行：{program:?} {args:?}");
    }
    let mut child = spawn(program, args)?;
    let stderr = Arc::new(Mutex::new(Tail::default()));
    let abort = Arc::new(AtomicBool::new(false));

    let stderr_reader = child.stderr.take().ok_or_else(|| "无法读取子进程错误输出".to_owned())?;
    let stderr_state = Arc::clone(&stderr);
    let stderr_abort = Arc::clone(&abort);
    let stderr_thread = thread::spawn(move || {
        let result = drain_stderr(stderr_reader, stderr_state);
        if result.is_err() {
            stderr_abort.store(true, Ordering::SeqCst);
        }
        result
    });
    let stdout_thread = child.stdout.take().map(|stdout| {
        let progress_abort = Arc::clone(&abort);
        let duration = progress_duration.filter(|d| d.is_finite() && *d > 0.0);
        thread::spawn(move || {
            let result = read_progress(stdout, duration);
            if result.is_err() {
                progress_abort.store(true, Ordering::SeqCst);
            }
            result
        })
    });
    let status = wait_child(&mut child, cancel, Some(&abort));
    let stderr_read = stderr_thread.join().unwrap_or_else(|_| Err("读取子进程错误输出时线程异常".into()));
    let progress_result = stdout_thread
        .map(|thread| thread.join().unwrap_or_else(|_| Err("读取子进程进度时线程异常".into())))
        .unwrap_or(Ok(()));

    if cancel.load(Ordering::SeqCst) {
        return Err("操作已取消".into());
    }
    stderr_read?;
    progress_result?;
    let status = status?;
    if !status.success() {
        let detail = stderr.lock().map(|tail| tail.text()).unwrap_or_default();
        return Err(exit_error(program, status, &detail));
    }
    Ok(())
}

pub fn capture(
    program: &Path,
    args: &[OsString],
    cancel: &Arc<AtomicBool>,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    read_stdout(program, args, cancel, move |reader| bounded_read(reader, max_bytes))
}

pub(crate) fn read_stdout<T, F>(
    program: &Path,
    args: &[OsString],
    cancel: &Arc<AtomicBool>,
    read: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(std::process::ChildStdout) -> Result<T, String> + Send + 'static,
{
    let mut child = spawn(program, args)?;
    let stderr = Arc::new(Mutex::new(Tail::default()));
    let abort = Arc::new(AtomicBool::new(false));

    let stderr_reader = child.stderr.take().ok_or_else(|| "无法读取子进程错误输出".to_owned())?;
    let stderr_state = Arc::clone(&stderr);
    let stderr_abort = Arc::clone(&abort);
    let stderr_thread = thread::spawn(move || {
        let result = drain_stderr(stderr_reader, stderr_state);
        if result.is_err() {
            stderr_abort.store(true, Ordering::SeqCst);
        }
        result
    });

    let stdout = child.stdout.take().ok_or_else(|| "无法读取子进程标准输出".to_owned())?;
    let stdout_abort = Arc::clone(&abort);
    let stdout_thread = thread::spawn(move || {
        let result = read(stdout);
        if result.is_err() { stdout_abort.store(true, Ordering::SeqCst); }
        result
    });
    let status = wait_child(&mut child, cancel, Some(&abort));
    let stdout_result = stdout_thread.join().unwrap_or_else(|_| Err("读取子进程输出时线程异常".into()));
    let stderr_read = stderr_thread.join().unwrap_or_else(|_| Err("读取子进程错误输出时线程异常".into()));

    if cancel.load(Ordering::SeqCst) {
        return Err("操作已取消".into());
    }
    let status = status?;
    stderr_read?;
    let bytes = stdout_result?;
    if !status.success() {
        let detail = stderr.lock().map(|tail| tail.text()).unwrap_or_default();
        return Err(exit_error(program, status, &detail));
    }
    Ok(bytes)
}

fn spawn(program: &Path, args: &[OsString]) -> Result<Child, String> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("无法启动 {}：{error}", program.display()))
}

fn wait_child(child: &mut Child, cancel: &AtomicBool, abort: Option<&AtomicBool>) -> Result<ExitStatus, String> {
    loop {
        if cancel.load(Ordering::SeqCst) {
            terminate_tree(child);
            let _ = child.wait();
            return Err("操作已取消".into());
        }
        if abort.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
            terminate_tree(child);
            let _ = child.wait();
            return Err("读取子进程管道失败或输出超过大小上限".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => thread::sleep(POLL_INTERVAL),
            Err(error) => {
                terminate_tree(child);
                let _ = child.wait();
                return Err(format!("等待子进程结束失败：{error}"));
            }
        }
    }
}

fn terminate_tree(child: &mut Child) {
    #[cfg(windows)]
    {
        let pid = child.id().to_string();
        let _ = Command::new("taskkill")
            .args([OsString::from("/PID"), OsString::from(pid), OsString::from("/T"), OsString::from("/F")])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}

fn drain_stderr<R: Read>(mut reader: R, tail: Arc<Mutex<Tail>>) -> Result<(), String> {
    let mut buffer = [0_u8; 8192];
    loop {
        let count = reader.read(&mut buffer).map_err(|error| format!("读取子进程错误输出失败：{error}"))?;
        if count == 0 {
            return Ok(());
        }
        if let Ok(mut state) = tail.lock() {
            state.push(&buffer[..count]);
        }
    }
}

fn bounded_read<R: Read>(mut reader: R, max_bytes: usize) -> Result<Vec<u8>, String> {
    let mut output = Vec::with_capacity(max_bytes.min(64 * 1024));
    let mut buffer = [0_u8; 8192];
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(count) => count,
            Err(error) => {
                return Err(format!("读取子进程输出失败：{error}"));
            }
        };
        if count == 0 {
            return Ok(output);
        }
        if output.len().saturating_add(count) > max_bytes {
            return Err(format!("子进程输出超过大小上限（{max_bytes} 字节）"));
        }
        output.extend_from_slice(&buffer[..count]);
    }
}

fn read_progress<R: Read>(reader: R, duration: Option<f64>) -> Result<(), String> {
    let mut reader = BufReader::new(reader);
    let mut line = Vec::with_capacity(128);
    let mut dropping = false;
    let mut last_report = Instant::now() - Duration::from_secs(2);
    let mut last_seconds = 0.0_f64;
    loop {
        let mut byte = [0_u8; 1];
        match reader.read(&mut byte) {
            Ok(0) => return Ok(()),
            Ok(_) => {
                if byte[0] == b'\n' {
                    if !dropping {
                        let text = String::from_utf8_lossy(&line);
                        if let Some(value) = text.strip_prefix("out_time_ms=") {
                            if let Ok(microseconds) = value.trim().parse::<u64>() {
                                last_seconds = last_seconds.max(microseconds as f64 / 1_000_000.0);
                            }
                        } else if text.trim() == "progress=end" {
                            if let Some(total) = duration {
                                eprintln!("进度：100% ({total:.1}/{total:.1} 秒)");
                            }
                            return Ok(());
                        }
                        if last_report.elapsed() >= Duration::from_secs(1) {
                            if let Some(total) = duration {
                                let percent = (last_seconds / total * 100.0).clamp(0.0, 99.0);
                                eprintln!("进度：{percent:.0}% ({:.1}/{total:.1} 秒)", last_seconds.min(total));
                            } else if last_seconds > 0.0 {
                                eprintln!("已处理 {:.1} 秒", last_seconds);
                            }
                            last_report = Instant::now();
                        }
                    }
                    line.clear();
                    dropping = false;
                } else if !dropping {
                    if line.len() < PROGRESS_LINE_LIMIT {
                        line.push(byte[0]);
                    } else {
                        line.clear();
                        dropping = true;
                    }
                }
            }
            Err(error) => return Err(format!("读取进度输出失败：{error}")),
        }
    }
}

fn exit_error(program: &Path, status: ExitStatus, detail: &str) -> String {
    let status_text = status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "被信号终止".to_owned());
    if detail.is_empty() {
        format!("{} 执行失败（退出码 {status_text}）", program.display())
    } else {
        format!("{} 执行失败（退出码 {status_text}）：\n{detail}", program.display())
    }
}
