//! `tests/media_safety.rs` —— 媒体管线安全测试（真实 FFmpeg 生成的小型夹具）。
//!
//! 覆盖 `scripts/media-smoke.ps1` 未覆盖的缺口（父代理指定的集成夹具取舍）：
//! 1. 非整数帧率源（30000/1001）→ 目标 60000/1001 的精确帧格（33→66 帧）、时长与尾帧保留；
//! 2. 输出/报告与源文件、既有报告的别名（精确路径、大小写变体、--overwrite 下）一律拒绝；
//! 3. 已存在的输出在输入缺失、损坏、截断、取消等失败路径下字节级保持原样；
//! 4. verify 对非整数帧率的精确校验（30000/1001 ≠ 30000/1，60 ≠ 60000/1001）。
//!
//! 运行命令（需要 ffmpeg/ffprobe；无网络、无硬编码睡眠、无全文件 RAM 夹具）：
//! ```powershell
//! $env:SMOOTHUP_FFMPEG  = ".tools/ffmpeg/ffmpeg-9.0.2-essentials_build/bin/ffmpeg.exe"
//! $env:SMOOTHUP_FFPROBE = ".tools/ffmpeg/ffmpeg-9.0.2-essentials_build/bin/ffprobe.exe"
//! cargo test --test media_safety -- --include-ignored --nocapture
//! ```
//! 未设置环境变量且 PATH 找不到工具时测试自动跳过，缺省 `cargo test` 保持绿色。
//!
//! 仅依赖 std + 平滑库自身依赖（serde_json 解析 ffprobe 输出，未新增依赖）。
//! 源文件保护用字节级快照对比，比哈希更严格。大小写别名探测仅在 Windows（NTFS
//! 大小写不敏感语义）执行。

use std::{
    env, fs,
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::AtomicBool,
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use smoothup::{
    media,
    types::{ConvertOptions, Engine, Platform, Quality, Rate, Tools},
};

const SKIP_HINT: &str = "缺少 ffmpeg/ffprobe：设置 SMOOTHUP_FFMPEG/SMOOTHUP_FFPROBE（.tools/ffmpeg/ffmpeg-9.0.2-essentials_build/bin/）或加入 PATH，然后运行 cargo test --test media_safety -- --include-ignored";

// ---------------------------------------------------------------- 工具解析

fn find_tool(env_name: &str, exe: &str) -> Option<PathBuf> {
    if let Ok(value) = env::var(env_name) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            let explicit = PathBuf::from(trimmed);
            if explicit.is_file() {
                return Some(explicit);
            }
        }
    }
    let path_var = env::var_os("PATH")?;
    env::split_paths(&path_var)
        .map(|dir| dir.join(exe))
        .find(|candidate| candidate.is_file())
}

fn tools_or_skip() -> Option<Tools> {
    let ffmpeg = find_tool("SMOOTHUP_FFMPEG", "ffmpeg.exe")?;
    let ffprobe = find_tool("SMOOTHUP_FFPROBE", "ffprobe.exe")?;
    Some(Tools { ffmpeg, ffprobe, rife: None, rife_model: None })
}

// ---------------------------------------------------------------- 进程与探测

struct ToolRun {
    ok: bool,
    stdout: String,
    stderr: String,
}

fn os(text: &str) -> OsString {
    OsString::from(text)
}

fn run_tool(program: &Path, args: &[OsString]) -> ToolRun {
    match Command::new(program).args(args).output() {
        Ok(output) => ToolRun {
            ok: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        },
        Err(err) => ToolRun {
            ok: false,
            stdout: String::new(),
            stderr: format!("无法启动 {program:?}: {err}"),
        },
    }
}

fn clip(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= 400 {
        trimmed.to_string()
    } else {
        format!("{}…", trimmed.chars().take(400).collect::<String>())
    }
}

fn probe_json(ffprobe: &Path, file: &Path) -> serde_json::Value {
    let run = run_tool(
        ffprobe,
        &[
            os("-v"),
            os("error"),
            os("-show_streams"),
            os("-show_format"),
            os("-of"),
            os("json"),
            file.as_os_str().to_os_string(),
        ],
    );
    assert!(run.ok, "ffprobe 探测 {file:?} 失败：{}", clip(&run.stderr));
    serde_json::from_str(&run.stdout)
        .unwrap_or_else(|err| panic!("ffprobe 输出不是合法 JSON：{err}；片段：{}", clip(&run.stdout)))
}

fn field_str<'a>(value: &'a serde_json::Value, pointer: &str) -> &'a str {
    value
        .pointer(pointer)
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("ffprobe 输出缺少字符串字段 {pointer}"))
}

fn field_u64(value: &serde_json::Value, pointer: &str) -> u64 {
    value
        .pointer(pointer)
        .and_then(|v| v.as_u64())
        .unwrap_or_else(|| panic!("ffprobe 输出缺少数值字段 {pointer}"))
}

fn duration_of(value: &serde_json::Value) -> f64 {
    field_str(value, "/format/duration")
        .parse::<f64>()
        .unwrap_or_else(|_| panic!("ffprobe 时长不是数字"))
}

fn frame_pts(ffprobe: &Path, file: &Path) -> Vec<f64> {
    let run = run_tool(
        ffprobe,
        &[
            os("-v"),
            os("error"),
            os("-select_streams"),
            os("v:0"),
            os("-show_entries"),
            os("frame=pts_time"),
            os("-of"),
            os("csv=p=0"),
            file.as_os_str().to_os_string(),
        ],
    );
    assert!(run.ok, "ffprobe 逐帧读取 {file:?} 失败：{}", clip(&run.stderr));
    run.stdout
        .lines()
        .filter_map(|line| line.trim().parse::<f64>().ok())
        .collect()
}

// ---------------------------------------------------------------- 夹具生成

/// NTSC 30000/1001 硬切夹具：0-14 帧纯红，15-32 帧纯绿，33 帧 ≈1.1011s，无音频。
fn generate_ntsc_cut_fixture(tools: &Tools, path: &Path) {
    let run = run_tool(
        &tools.ffmpeg,
        &[
            os("-y"),
            os("-v"),
            os("error"),
            os("-f"),
            os("lavfi"),
            os("-i"),
            os("color=c=red:size=128x96:rate=30000/1001"),
            os("-f"),
            os("lavfi"),
            os("-i"),
            os("color=c=green:size=128x96:rate=30000/1001"),
            os("-filter_complex"),
            os("[0:v]trim=end_frame=15,setpts=PTS-STARTPTS[r];[1:v]trim=end_frame=18,setpts=PTS-STARTPTS[g];[r][g]concat=n=2:v=1:a=0[v]"),
            os("-map"),
            os("[v]"),
            os("-c:v"),
            os("libx264"),
            os("-preset"),
            os("ultrafast"),
            os("-crf"),
            os("18"),
            os("-pix_fmt"),
            os("yuv420p"),
            path.as_os_str().to_os_string(),
        ],
    );
    assert!(run.ok, "NTSC 夹具生成失败 {path:?}：{}", clip(&run.stderr));
}

/// 60fps 1 秒含音频夹具。
fn generate_60fps_fixture(tools: &Tools, path: &Path) {
    let run = run_tool(
        &tools.ffmpeg,
        &[
            os("-y"),
            os("-v"),
            os("error"),
            os("-f"),
            os("lavfi"),
            os("-i"),
            os("testsrc2=size=160x120:rate=60:duration=1"),
            os("-f"),
            os("lavfi"),
            os("-i"),
            os("sine=frequency=800:sample_rate=48000:duration=1"),
            os("-c:v"),
            os("libx264"),
            os("-preset"),
            os("ultrafast"),
            os("-crf"),
            os("20"),
            os("-pix_fmt"),
            os("yuv420p"),
            os("-c:a"),
            os("aac"),
            os("-b:a"),
            os("96k"),
            os("-shortest"),
            path.as_os_str().to_os_string(),
        ],
    );
    assert!(run.ok, "60fps 夹具生成失败 {path:?}：{}", clip(&run.stderr));
}

// ---------------------------------------------------------------- 断言辅助

fn options(input: &Path, output: &Path, overwrite: bool) -> ConvertOptions {
    ConvertOptions {
        input: input.to_path_buf(),
        output: output.to_path_buf(),
        platform: Platform::Douyin,
        fps: Rate::new(60, 1).expect("60/1 应有效"),
        engine: Engine::Cpu,
        quality: Quality::Balanced,
        preview: None,
        report: None,
        verbose: false,
        overwrite,
    }
}

fn snapshot(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap_or_else(|err| panic!("读取 {path:?} 失败：{err}"))
}

fn assert_unchanged(path: &Path, before: &[u8], context: &str) {
    let after =
        fs::read(path).unwrap_or_else(|err| panic!("{context}：文件读取失败：{err}"));
    assert_eq!(after, before, "{context}：文件内容被改变");
}

fn must_reject(
    result: Result<PathBuf, String>,
    what: &str,
) {
    if let Ok(published) = result {
        panic!("{what} 被接受并发布到 {published:?}，这是破坏性/别名漏洞");
    }
}

struct WorkDir(PathBuf);

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn work_dir(tag: &str) -> WorkDir {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间早于 UNIX_EPOCH")
        .as_nanos();
    let path = env::temp_dir().join(format!(
        "smoothup-media-safety-{tag}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("创建临时工作目录");
    WorkDir(path)
}

// ---------------------------------------------------------------- 测试 1

/// 非整数源 → 60000/1001 精确帧格/时长/尾帧 + 输出与报告别名防覆盖全链路。
#[test]
#[ignore = "需要 ffmpeg/ffprobe（SMOOTHUP_FFMPEG/SMOOTHUP_FFPROBE 或 PATH）：cargo test --test media_safety ntsc_source_survives_alias_report_and_keeps_exact_tail -- --ignored --nocapture"]
fn ntsc_source_survives_alias_report_and_keeps_exact_tail() {
    let Some(tools) = tools_or_skip() else {
        eprintln!("{SKIP_HINT}");
        return;
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let work = work_dir("ntsc");
    let dir = work.0.clone();

    let source = dir.join("source-ntsc.mp4");
    generate_ntsc_cut_fixture(&tools, &source);

    // 夹具预检：确认为 30000/1001、33 帧、约 1.1011s。
    let src = probe_json(&tools.ffprobe, &source);
    assert_eq!(
        field_str(&src, "/streams/0/avg_frame_rate"),
        "30000/1001",
        "夹具源应为 NTSC 30000/1001"
    );
    assert_eq!(field_str(&src, "/streams/0/nb_frames"), "33", "夹具源应为 33 帧");
    let source_duration = duration_of(&src);
    assert!(
        (source_duration - 1.1011).abs() < 0.02,
        "夹具源时长异常：{source_duration}"
    );
    let source_snap = snapshot(&source);

    let output = dir.join("source-ntsc.douyin.60000-1001fps.mp4");
    let report = dir.join("report.json");

    // -- 1) 输出别名 = 源文件：必须拒绝，源字节不变。
    let alias_source_output = options(&source, &source, false);
    must_reject(
        media::convert(&tools, &alias_source_output, &cancel),
        "把输出指向源文件",
    );
    assert_unchanged(&source, &source_snap, "输出别名指向源文件后");

    // -- 2) 报告别名 = 源文件：必须拒绝，源不变，且不得有部分输出落盘。
    let mut alias_report_source = options(&source, &output, false);
    alias_report_source.report = Some(source.clone());
    must_reject(
        media::convert(&tools, &alias_report_source, &cancel),
        "把报告指向源文件",
    );
    assert_unchanged(&source, &source_snap, "报告别名指向源文件后");
    assert!(
        !output.exists(),
        "别名失败后不应有部分输出落盘：{}",
        output.display()
    );

    // -- 3) 大小写变体输出别名（仅 Windows；NTFS 大小写不敏感）。
    let alias_upper = dir.join("SOURCE-NTSC.MP4");
    if cfg!(windows) {
        if fs::copy(&source, &alias_upper).is_err() {
            assert!(
                fs::metadata(&alias_upper).is_ok(),
                "无法准备大小写别名：{}",
                alias_upper.display()
            );
        }
        let alias_opts = options(&source, &alias_upper, false);
        must_reject(
            media::convert(&tools, &alias_opts, &cancel),
            "大小写变体输出别名",
        );
        assert_unchanged(&source, &source_snap, "大小写变体别名转换后");
    }

    // -- 4) 正式转换：30000/1001 → 60000/1001，CPU 引擎，写报告。
    assert_unchanged(&source, &source_snap, "正式转换前");
    let mut convert = options(&source, &output, false);
    convert.fps = Rate::new(60000, 1001).expect("60000/1001 应有效");
    convert.report = Some(report.clone());
    let published = match media::convert(&tools, &convert, &cancel) {
        Ok(path) => path,
        Err(err) => panic!("NTSC 源 60000/1001 转换失败：{err}"),
    };
    assert_eq!(published, output, "发布路径应与请求输出一致");
    assert!(output.exists(), "转换未生成输出");
    assert!(report.exists(), "转换未生成报告");
    assert_unchanged(&source, &source_snap, "成功转换后");

    // -- 5) 输出帧格：60000/1001 必须是源的精确两倍帧格（33→66），时长不漂移。
    let out = probe_json(&tools.ffprobe, &output);
    assert_eq!(field_str(&out, "/streams/0/codec_name"), "h264", "输出编码应为 H264");
    assert_eq!(field_str(&out, "/streams/0/pix_fmt"), "yuv420p", "输出像素格式应为 yuv420p");
    assert_eq!(field_u64(&out, "/streams/0/width"), 128, "输出宽度不应被强制缩放");
    assert_eq!(field_u64(&out, "/streams/0/height"), 96, "输出高度不应被强制缩放");
    assert_eq!(
        field_str(&out, "/streams/0/avg_frame_rate"),
        "60000/1001",
        "输出帧率必须精确为 60000/1001（出现 60/1 说明有理数目标被十进制化）"
    );
    assert_eq!(
        field_str(&out, "/streams/0/nb_frames"),
        "66",
        "60000/1001 输出必须是 33 源帧的精确两倍帧格 66 帧；帧数不符说明时间轴漂移或丢帧"
    );
    let out_duration = duration_of(&out);
    assert!(
        (out_duration - source_duration).abs() <= 0.05,
        "输出时长 {out_duration} 与源时长 {source_duration} 偏差超过 0.05s（尾部丢帧或时基漂移）"
    );
    let start: f64 = field_str(&out, "/streams/0/start_time")
        .parse()
        .unwrap_or(f64::NAN);
    assert!(start.abs() <= 0.05, "输出起始时间异常：{start}");

    // -- 6) 尾帧保留：逐帧 PTS 覆盖到接近末尾（minterpolate EOF 冲刷）。
    let pts = frame_pts(&tools.ffprobe, &output);
    assert!(
        pts.len() >= 66,
        "逐帧 PTS 条目数 {} 少于 66，逐帧读取不完整",
        pts.len()
    );
    let last = pts[pts.len() - 1];
    assert!(
        last >= source_duration - 0.1,
        "最后一帧 PTS {last} 过早（应接近 {source_duration}），minterpolate 尾部/EOF 冲刷丢失"
    );

    // -- 7) 报告可解析、非空。
    let report_before = snapshot(&report);
    serde_json::from_str::<serde_json::Value>(&String::from_utf8_lossy(&report_before))
        .expect("报告不是合法 JSON");

    // -- 8) 既有报告作为输出/作为报告别名：必须拒绝且报告内容不变。
    let mut report_as_output = options(&source, &report.clone(), false);
    must_reject(
        media::convert(&tools, &report_as_output, &cancel),
        "把输出指向既有报告",
    );
    assert_unchanged(&report, &report_before, "输出指向既有报告后");

    let mut report_as_report = options(&source, &output, false);
    report_as_report.report = Some(report.clone());
    must_reject(
        media::convert(&tools, &report_as_report, &cancel),
        "把报告指向既有报告",
    );
    assert_unchanged(&report, &report_before, "报告别名指向既有报告后");

    // -- 9) --overwrite 下大小写变体报告别名也必须拒绝（报告不得被改写）。
    if cfg!(windows) {
        let report_upper = dir.join("REPORT.JSON");
        if fs::copy(&report, &report_upper).is_err() {
            assert!(
                fs::metadata(&report_upper).is_ok(),
                "无法准备报告大小写别名：{}",
                report_upper.display()
            );
        }
        let report_upper_snap = snapshot(&report_upper);
        let mut overwrite_alias = options(&source, &report_upper, true);
        overwrite_alias.report = Some(report_upper.clone());
        must_reject(
            media::convert(&tools, &overwrite_alias, &cancel),
            "--overwrite 下大小写变体报告别名",
        );
        assert_unchanged(&report, &report_before, "--overwrite 别名探测后原报告");
        assert_unchanged(&report_upper, &report_upper_snap, "--overwrite 别名探测后别名副本");
    }

    // -- 10) 终态：源文件字节不变。
    assert_unchanged(&source, &source_snap, "全部操作结束后");
}

// ---------------------------------------------------------------- 测试 2

/// 已存在输出在失败路径（缺失/损坏/截断输入、取消）下保持原样；verify 精确帧率校验。
#[test]
#[ignore = "需要 ffmpeg/ffprobe（SMOOTHUP_FFMPEG/SMOOTHUP_FFPROBE 或 PATH）：cargo test --test media_safety existing_output_and_fps_gates -- --ignored --nocapture"]
fn existing_output_and_fps_gates() {
    let Some(tools) = tools_or_skip() else {
        eprintln!("{SKIP_HINT}");
        return;
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::new(AtomicBool::new(true));
    let work = work_dir("gates");
    let dir = work.0.clone();

    // 守护输出：任何失败路径都不得改写它。
    let guard = dir.join("guard-output.mp4");
    let guard_content = "不可覆盖的既有输出占位内容";
    fs::write(&guard, guard_content).expect("写入守护输出");
    let guard_snap = snapshot(&guard);

    // -- 1) 输入不存在：拒绝且守护输出不变。
    let missing = dir.join("missing-input.mp4");
    let opts = options(&missing, &guard, false);
    must_reject(media::convert(&tools, &opts, &cancel), "输入不存在时的转换");
    assert_unchanged(&guard, &guard_snap, "输入缺失失败后");

    // -- 2) 输入是文本（损坏/非媒体）：拒绝且守护输出不变。
    let garbage = dir.join("garbage.mp4");
    fs::write(&garbage, "这不是视频文件，只是占位文本").expect("写入损坏夹具");
    let opts = options(&garbage, &guard, false);
    must_reject(media::convert(&tools, &opts, &cancel), "损坏输入的转换");
    assert_unchanged(&guard, &guard_snap, "损坏输入失败后");

    // -- 3) 截断 MP4（moov 缺失）：拒绝、目标不得出现、守护输出不变。
    let b_mp4 = dir.join("input-60fps.mp4");
    generate_60fps_fixture(&tools, &b_mp4);
    let b_probe = probe_json(&tools.ffprobe, &b_mp4);
    assert_eq!(
        field_str(&b_probe, "/streams/0/avg_frame_rate"),
        "60/1",
        "60fps 夹具预检失败"
    );
    let full = fs::read(&b_mp4).expect("读取 60fps 夹具");
    let truncated = dir.join("truncated.mp4");
    fs::write(&truncated, &full[..full.len() / 2]).expect("写入截断夹具");
    let fresh_target = dir.join("should-never-exist.mp4");
    let opts = options(&truncated, &fresh_target, false);
    must_reject(media::convert(&tools, &opts, &cancel), "截断输入的转换");
    assert!(
        !fresh_target.exists(),
        "截断输入失败后不应生成输出：{}",
        fresh_target.display()
    );
    assert_unchanged(&guard, &guard_snap, "截断输入失败后");

    // -- 4) 预先置位的取消令牌（即使 --overwrite）：拒绝且守护输出不变。
    let opts = options(&b_mp4, &guard, true);
    must_reject(media::convert(&tools, &opts, &cancelled), "取消令牌置位时的转换");
    assert_unchanged(&guard, &guard_snap, "取消失败后（含 --overwrite 场景）");

    // -- 5) verify 帧率门：非整数帧率必须精确匹配，不能按十进制近似。
    let ntsc = dir.join("source-ntsc.mp4");
    generate_ntsc_cut_fixture(&tools, &ntsc);
    match media::verify(&tools, &ntsc, Some(Rate::new(60, 1).unwrap()), &cancel, false) {
        Ok(_) => panic!("verify 应拒绝把 29.97fps（30000/1001）源当作 60fps"),
        Err(_) => {}
    }
    match media::verify(&tools, &ntsc, Some(Rate::new(30000, 1).unwrap()), &cancel, false) {
        Ok(_) => panic!("verify 必须区分 30000/1001 与 30000/1，不能按十进制近似通过"),
        Err(_) => {}
    }
    match media::verify(&tools, &ntsc, Some(Rate::new(30000, 1001).unwrap()), &cancel, false) {
        Ok(_) => {}
        Err(err) => panic!("verify 应接受精确 30000/1001 源：{err}"),
    }
    match media::verify(&tools, &b_mp4, Some(Rate::new(60000, 1001).unwrap()), &cancel, false) {
        Ok(_) => panic!("verify 应拒绝把 60/1 源当作 60000/1001"),
        Err(_) => {}
    }
    match media::verify(&tools, &b_mp4, Some(Rate::new(60, 1).unwrap()), &cancel, false) {
        Ok(_) => {}
        Err(err) => panic!("verify 应接受精确 60/1 源：{err}"),
    }
    assert_unchanged(&guard, &guard_snap, "verify 阶段后");
}
