use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{process, types::{ConvertOptions, Engine, Platform, Quality, Rate, Tools}};

#[path = "media/rife.rs"]
mod rife;

const PROBE_LIMIT: usize = 8 * 1024 * 1024;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MediaInfo {
    pub path: String,
    pub format_name: Option<String>,
    pub format_start_time: f64,
    pub duration_seconds: Option<f64>,
    pub size_bytes: Option<u64>,
    pub bit_rate: Option<u64>,
    pub video: Option<VideoInfo>,
    pub audio_streams: Vec<AudioInfo>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct VideoInfo {
    pub codec_name: Option<String>,
    pub profile: Option<String>,
    pub time_base: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub pixel_format: Option<String>,
    pub sample_aspect_ratio: Option<String>,
    pub display_aspect_ratio: Option<String>,
    pub frame_rate: Option<String>,
    pub real_frame_rate: Option<String>,
    pub average_frame_rate: Option<String>,
    pub frame_count: Option<u64>,
    pub start_time: Option<f64>,
    pub duration_seconds: Option<f64>,
    pub color_primaries: Option<String>,
    pub color_transfer: Option<String>,
    pub color_space: Option<String>,
    pub rotation_degrees: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AudioInfo {
    pub codec_name: Option<String>,
    pub sample_rate: Option<String>,
    pub channels: Option<u32>,
    pub channel_layout: Option<String>,
    pub start_time: Option<f64>,
    pub duration_seconds: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Verification {
    pub path: String,
    pub valid: bool,
    pub expected_frame_rate: Option<Rate>,
    pub actual_frame_rate: Option<Rate>,
    pub duration_seconds: Option<f64>,
    pub video_codec: Option<String>,
    pub pixel_format: Option<String>,
    pub decoded_successfully: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Deserialize)]
struct ProbeOutput {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    format: Option<ProbeFormat>,
}

#[derive(Deserialize)]
struct ProbeFormat {
    format_name: Option<String>,
    duration: Option<String>,
    size: Option<String>,
    bit_rate: Option<String>,
    start_time: Option<String>,
}

#[derive(Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    time_base: Option<String>,
    profile: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    pix_fmt: Option<String>,
    sample_aspect_ratio: Option<String>,
    display_aspect_ratio: Option<String>,
    r_frame_rate: Option<String>,
    avg_frame_rate: Option<String>,
    nb_frames: Option<String>,
    start_time: Option<String>,
    duration: Option<String>,
    color_primaries: Option<String>,
    color_transfer: Option<String>,
    color_space: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
    channel_layout: Option<String>,
    side_data_list: Option<Vec<ProbeSideData>>,
}

#[derive(Deserialize)]
struct ProbeSideData {
    rotation: Option<f64>,
}

pub fn inspect(tools: &Tools, input: &Path, cancel: &Arc<AtomicBool>) -> Result<MediaInfo, String> {
    let path = input.canonicalize().map_err(|e| format!("无法读取输入文件 {}：{e}", input.display()))?;
    if !path.is_file() {
        return Err(format!("输入路径不是文件：{}", path.display()));
    }
    let probe = probe(tools, &path, cancel)?;
    media_info(&path, probe)
}


#[derive(Default)]
struct FrameTiming {
    count: u64,
    first: i64,
    last: i64,
    max_step: i64,
    cfr: bool,
}

fn frame_timing(tools: &Tools, input: &Path, video: &VideoInfo, cancel: &Arc<AtomicBool>) -> Result<FrameTiming, String> {
    let time_base = parse_rate(video.time_base.as_deref()).ok_or("视频时间基准无效")?;
    let rate = video_rate(video).ok_or("无法确定视频帧率")?;
    let args = [
        OsString::from("-v"), OsString::from("error"),
        OsString::from("-select_streams"), OsString::from("v:0"),
        OsString::from("-show_entries"), OsString::from("frame=best_effort_timestamp"),
        OsString::from("-of"), OsString::from("csv=p=0"),
        input.as_os_str().to_owned(),
    ];
    process::read_stdout(&tools.ffprobe, &args, cancel, move |stdout| {
        let mut reader = BufReader::new(stdout);
        let mut line = String::with_capacity(128);
        let mut timing = FrameTiming { cfr: true, ..FrameTiming::default() };
        let tick_units = i128::from(time_base.num) * i128::from(rate.num);
        let frame_units = i128::from(time_base.den) * i128::from(rate.den);
        loop {
            line.clear();
            let bytes = reader.by_ref().take(4097).read_line(&mut line)
                .map_err(|e| format!("读取逐帧时间戳失败：{e}"))?;
            if bytes == 0 { break; }
            if bytes > 4096 { return Err("ffprobe 时间戳行过长".into()); }
            let token = line.trim().split(',').next().unwrap_or_default();
            if token.is_empty() { continue; }
            let pts = token.parse::<i64>().map_err(|_| format!("无效视频帧时间戳：{token:?}"))?;
            if timing.count == 0 {
                timing.first = pts;
            } else {
                let step = pts.checked_sub(timing.last).filter(|step| *step > 0)
                    .ok_or("视频帧时间戳未严格递增")?;
                timing.max_step = timing.max_step.max(step);
                let elapsed = i128::from(pts) - i128::from(timing.first);
                let error = elapsed * tick_units - i128::from(timing.count) * frame_units;
                if error.abs() > tick_units { timing.cfr = false; }
            }
            timing.last = pts;
            timing.count = timing.count.checked_add(1).ok_or("视频帧数溢出")?;
        }
        if timing.count == 0 { return Err("视频中未解码到有效帧".into()); }
        Ok(timing)
    })
}

pub fn verify(
    tools: &Tools,
    input: &Path,
    expected: Option<Rate>,
    cancel: &Arc<AtomicBool>,
    verbose: bool,
) -> Result<Verification, String> {
    let info = inspect(tools, input, cancel)?;
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let video = info.video.as_ref();
    if video.is_none() { errors.push("文件中没有视频流".to_owned()); }
    if !info.duration_seconds.is_some_and(|duration| duration.is_finite() && duration > 0.0) {
        errors.push("媒体时长无效或缺失".to_owned());
    }
    let actual_rate = video.and_then(video_rate);
    let timing = video.map(|video| frame_timing(tools, input, video, cancel)).transpose();
    let timing = match timing {
        Ok(timing) => timing,
        Err(error) if cancel.load(Ordering::SeqCst) => return Err(error),
        Err(error) => { errors.push(error); None }
    };
    if let Some(timing) = &timing {
        if let Some(declared) = video.and_then(|video| video.frame_count) {
            if timing.count != declared { errors.push(format!("解码帧数 {} 与声明帧数 {declared} 不符", timing.count)); }
        }
        if let Some(expected) = expected {
            if !actual_rate.is_some_and(|rate| rate_eq(rate, expected)) {
                errors.push(format!("帧率不匹配：预期 {expected}"));
            }
            if !timing.cfr { errors.push("逐帧时间戳与标称帧率不符，文件为可变帧率".into()); }
        } else if !timing.cfr {
            warnings.push("视频为可变帧率；指定 --fps 可检查恒定帧率".into());
        }
        if let (Some(duration), Some(time_base), Some(rate)) = (
            video.and_then(|v| v.duration_seconds),
            video.and_then(|v| parse_rate(v.time_base.as_deref())), actual_rate,
        ) {
            let decoded_duration = (timing.last as f64 - timing.first as f64) * time_base.as_f64() + 1.0 / rate.as_f64();
            if (decoded_duration - duration).abs() > (2.0 / rate.as_f64()).max(0.05) {
                errors.push(format!("解码时长 {decoded_duration:.3} 与视频流时长 {duration:.3} 不符"));
            }
        }
    }
    if video.is_some_and(|video| video.codec_name.is_none()) {
        errors.push("无法确定视频编码格式".to_owned());
    }
    if video.is_some_and(|video| video.width.unwrap_or(0) == 0 || video.height.unwrap_or(0) == 0) {
        errors.push("视频尺寸无效".to_owned());
    }
    if verbose { eprintln!("正在完整解码检查媒体流…"); }
    let args = ffmpeg_args([
        OsString::from("-v"), OsString::from("error"), OsString::from("-xerror"),
        OsString::from("-i"), input.as_os_str().to_owned(),
        OsString::from("-map"), OsString::from("0:v:0"),
        OsString::from("-map"), OsString::from("0:a?"),
        OsString::from("-f"), OsString::from("null"), OsString::from("-"),
    ]);
    let decoded = process::run(&tools.ffmpeg, &args, cancel, verbose, info.duration_seconds).is_ok();
    if !decoded {
        if cancel.load(Ordering::SeqCst) { return Err("操作已取消".into()); }
        errors.push("完整解码失败或 FFmpeg 报告媒体损坏".to_owned());
    }
    if info.audio_streams.is_empty() { warnings.push("未检测到音频流".to_owned()); }
    let valid = errors.is_empty();
    Ok(Verification {
        path: input.to_string_lossy().into_owned(), valid, expected_frame_rate: expected,
        actual_frame_rate: actual_rate, duration_seconds: video.and_then(|v| v.duration_seconds).or(info.duration_seconds),
        video_codec: video.and_then(|v| v.codec_name.clone()),
        pixel_format: video.and_then(|v| v.pixel_format.clone()),
        decoded_successfully: decoded, errors, warnings,
    })
}

pub fn convert(tools: &Tools, options: &ConvertOptions, cancel: &Arc<AtomicBool>) -> Result<PathBuf, String> {
    validate_convert_options(options)?;
    let input = options.input.canonicalize().map_err(|e| format!("无法读取输入文件 {}：{e}", options.input.display()))?;
    if !input.is_file() {
        return Err(format!("输入路径不是文件：{}", input.display()));
    }
    let output = absolute_target(&options.output)?;
    let report_path = options.report.as_deref().map(absolute_target).transpose()?;
    if let Some(report) = &report_path {
        if same_path(report, &input) || same_path(report, &output) {
            return Err("报告路径不能与输入或输出文件相同".into());
        }
    }
    if same_path(&input, &output) {
        return Err("输出文件不能覆盖源文件".into());
    }
    ensure_publish_allowed(&output, options.overwrite)?;
    if let Some(report) = &report_path {
        ensure_publish_allowed(report, options.overwrite)?;
    }
    check_cancel(cancel)?;

    let source = inspect(tools, &input, cancel)?;
    let video = source.video.as_ref().ok_or_else(|| "输入媒体没有视频流".to_owned())?;
    let source_duration = video.duration_seconds.or(source.duration_seconds)
        .ok_or_else(|| "无法确定输入视频时长".to_owned())?;
    if !source_duration.is_finite() || source_duration <= 0.0 {
        return Err("输入视频时长无效".into());
    }
    let duration = options.preview.map_or(source_duration, |seconds| seconds.min(source_duration));
    if !duration.is_finite() || duration <= 0.0 {
        return Err("预览时长无效".into());
    }
    let source_rate = video_rate(video).ok_or("无法确定输入帧率")?;
    let timing = frame_timing(tools, &input, video, cancel)?;
    let cfr = is_cfr(video) && timing.cfr;
    let needs_interpolation = rate_lt(source_rate, options.fps);
    let engine = if options.engine == Engine::Auto {
        if cfr && tools.rife.is_some() && tools.rife_model.is_some() { Engine::Rife } else { Engine::Cpu }
    } else { options.engine };
    if engine == Engine::Rife && needs_interpolation {
        if tools.rife.is_none() || tools.rife_model.is_none() {
            return Err("未找到 RIFE 程序或模型；请运行 smoothup setup --rife".into());
        }
        if !cfr { return Err("RIFE 仅支持恒定帧率输入；请使用 --engine cpu".into()); }
    }

    let workspace = TempWorkspace::create(output.parent().unwrap_or_else(|| Path::new(".")))?;
    let staged_output = StagedFile::create(&output, "mp4")?;
    let output_duration = rounded_duration(duration, options.fps)?;
    let input_start = video.start_time.unwrap_or(source.format_start_time);
    let hdr = is_hdr(video);
    let base_filters = video_filters(video, hdr)?;
    let can_copy = options.preview.is_none()
        && !needs_interpolation
        && cfr && rate_eq(options.fps, source_rate)
        && copy_eligible(&source, video)
        && !hdr
        && base_filters.is_empty();

    eprintln!("正在检查输入媒体…");
    if can_copy {
        let mut args = vec![
            OsString::from("-copyts"),
            OsString::from("-itsoffset"), OsString::from(format_seconds(-input_start)),
            OsString::from("-i"), input.as_os_str().to_owned(),
            OsString::from("-map"), OsString::from("0:v:0"),
            OsString::from("-map"), OsString::from("0:a?"),
            OsString::from("-c"), OsString::from("copy"),
            OsString::from("-map_metadata"), OsString::from("0"),
            OsString::from("-movflags"), OsString::from("+faststart"),
            OsString::from("-y"), staged_output.path.as_os_str().to_owned(),
        ];
        args.extend(progress_args());
        eprintln!("源视频已达到目标帧率，采用无损流复制。");
        run_ffmpeg(tools, &args, cancel, options.verbose, Some(duration))?;
    } else if needs_interpolation && engine == Engine::Rife {
        let frame_count = timing.count;
        let intermediate = workspace.path.join("rife-video.mp4");
        let filters = base_filters.join(",");
        eprintln!("正在使用 RIFE GPU 运动插帧…");
        rife::render(
            tools,
            rife::RenderRequest {
                input: &input, destination: &intermediate, workspace: &workspace.path,
                video, source_rate, target_rate: options.fps, frame_count, duration,
                format_start_time: source.format_start_time,
                filters: &filters, quality: options.quality, verbose: options.verbose,
            },
            cancel,
        )?;
        mux_generated_video(
            tools, &intermediate, &staged_output.path,
            &source, output_duration, options, cancel,
        )?;
    } else {
        let mut filters = base_filters;
        if needs_interpolation {
            let mode = match options.quality {
                Quality::Balanced => "me=epzs",
                Quality::High => "me=umh",
            };
            filters.push(format!(
                "tpad=stop_mode=clone:stop_duration={},minterpolate=fps={}:mi_mode=mci:mc_mode=obmc:me_mode=bidir:{mode}:scd=fdiff:scd_threshold=10",
                format_seconds(2.0 * (timing.max_step as f64 * parse_rate(video.time_base.as_deref()).ok_or("视频时基无效")?.as_f64()).max(1.0 / source_rate.as_f64()) + 1.0 / options.fps.as_f64()),
                rate_string(options.fps),
            ));
            eprintln!("正在使用 CPU 运动补偿插帧…");
        } else {
            if !rate_eq(options.fps, source_rate) || !cfr {
                filters.push(format!("fps=fps={}:round=near", rate_string(options.fps)));
            }
            eprintln!("正在转换并规范化视频帧率…");
        }
        filters.push(format!("trim=duration={}", format_seconds(output_duration)));
        let filter = filters.join(",");
        let mut args = vec![
            OsString::from("-copyts"),
            OsString::from("-itsoffset"), OsString::from(format_seconds(-input_start)),
            OsString::from("-i"), input.as_os_str().to_owned(),
            OsString::from("-map"), OsString::from("0:v:0"),
            OsString::from("-map"), OsString::from("0:a?"),
            OsString::from("-vf"), OsString::from(filter),
            OsString::from("-c:v"), OsString::from("libx264"),
        ];
        args.extend(encode_settings(options.quality));
        args.extend([
            OsString::from("-pix_fmt"), OsString::from("yuv420p"),
            OsString::from("-r"), OsString::from(rate_string(options.fps)),
            OsString::from("-fps_mode:v"), OsString::from("cfr"),
            OsString::from("-video_track_timescale"), OsString::from(options.fps.num.to_string()),
            OsString::from("-color_primaries"), OsString::from("bt709"),
            OsString::from("-color_trc"), OsString::from("bt709"),
            OsString::from("-colorspace"), OsString::from("bt709"),
            OsString::from("-c:a"), OsString::from("aac"),
            OsString::from("-b:a"), OsString::from("192k"),
            OsString::from("-ar"), OsString::from("48000"),
            OsString::from("-map_metadata"), OsString::from("0"),
            OsString::from("-t"), OsString::from(format_seconds(output_duration)),
            OsString::from("-movflags"), OsString::from("+faststart"),
            OsString::from("-y"), staged_output.path.as_os_str().to_owned(),
        ]);
        args.extend(progress_args());
        run_ffmpeg(tools, &args, cancel, options.verbose, Some(duration))?;
    }

    check_cancel(cancel)?;
    eprintln!("正在完整解码并校验临时输出…");
    let mut verification = verify(tools, &staged_output.path, Some(options.fps), cancel, options.verbose)?;
    if !verification.valid {
        return Err(format!("输出校验失败：{}", verification.errors.join("；")));
    }
    if let Some(actual_duration) = verification.duration_seconds {
        let tolerance = f64::from(options.fps.den) / f64::from(options.fps.num) * 1.5;
        if (actual_duration - output_duration).abs() > tolerance {
            return Err(format!("输出时长校验失败：预期约 {:.3} 秒，实际 {:.3} 秒", output_duration, actual_duration));
        }
    }
    verification.path = output.to_string_lossy().into_owned();

    let report_temp = if let Some(path) = &report_path {
        Some(write_report_temp(path, &source, &verification, options, engine, duration, output_duration)?)
    } else {
        None
    };
    check_cancel(cancel)?;
    ensure_publish_allowed(&output, options.overwrite)?;
    staged_output.publish(&output, options.overwrite)?;
    if let (Some(report_path), Some(report_temp)) = (&report_path, report_temp) {
        if let Err(error) = report_temp.publish(report_path, options.overwrite) {
            return Err(format!("输出视频已校验并发布，但报告文件发布失败：{error}"));
        }
    }
    eprintln!("转换完成：{}", output.display());
    Ok(output)
}

fn validate_convert_options(options: &ConvertOptions) -> Result<(), String> {
    if options.preview.is_some_and(|v| !v.is_finite() || v <= 0.0) {
        return Err("预览时长必须是有限的正数".into());
    }
    if options.fps.num == 0 || options.fps.den == 0 {
        return Err("目标帧率无效".into());
    }
    if options.input.as_os_str().is_empty() || options.output.as_os_str().is_empty() {
        return Err("输入或输出路径不能为空".into());
    }
    Ok(())
}

fn probe(tools: &Tools, path: &Path, cancel: &Arc<AtomicBool>) -> Result<ProbeOutput, String> {
    let args = ffmpeg_args([
        OsString::from("-v"), OsString::from("error"),
        OsString::from("-show_format"), OsString::from("-show_streams"),
        OsString::from("-of"), OsString::from("json"),
        path.as_os_str().to_owned(),
    ]);
    let bytes = process::capture(&tools.ffprobe, &args, cancel, PROBE_LIMIT)?;
    serde_json::from_slice(&bytes).map_err(|e| format!("无法解析 ffprobe 媒体信息：{e}"))
}

fn media_info(path: &Path, probe: ProbeOutput) -> Result<MediaInfo, String> {
    let format = probe.format.unwrap_or(ProbeFormat {
        format_name: None, duration: None, size: None, bit_rate: None, start_time: None,
    });
    let streams = probe.streams;
    let video_stream = streams.iter().find(|s| s.codec_type.as_deref() == Some("video"));
    let video = video_stream.map(|s| VideoInfo {
        codec_name: s.codec_name.clone(),
        time_base: s.time_base.clone(),
        profile: s.profile.clone(),
        width: s.width,
        height: s.height,
        pixel_format: s.pix_fmt.clone(),
        sample_aspect_ratio: s.sample_aspect_ratio.clone(),
        display_aspect_ratio: s.display_aspect_ratio.clone(),
        frame_rate: s.r_frame_rate.clone(),
        real_frame_rate: s.r_frame_rate.clone(),
        average_frame_rate: s.avg_frame_rate.clone(),
        frame_count: s.nb_frames.as_deref().and_then(|v| v.parse().ok()),
        start_time: s.start_time.as_deref().and_then(parse_f64),
        duration_seconds: s.duration.as_deref().and_then(parse_f64),
        color_primaries: s.color_primaries.clone(),
        color_transfer: s.color_transfer.clone(),
        color_space: s.color_space.clone(),
        rotation_degrees: s.side_data_list.as_ref().and_then(|data| data.iter().find_map(|entry| entry.rotation)),
    });
    let audio_streams = streams.iter().filter(|s| s.codec_type.as_deref() == Some("audio")).map(|s| AudioInfo {
        codec_name: s.codec_name.clone(),
        sample_rate: s.sample_rate.clone(),
        channels: s.channels,
        channel_layout: s.channel_layout.clone(),
        start_time: s.start_time.as_deref().and_then(parse_f64),
        duration_seconds: s.duration.as_deref().and_then(parse_f64),
    }).collect();
    let duration_seconds = format.duration.as_deref().and_then(parse_f64)
        .or_else(|| streams.iter().filter_map(|s| s.duration.as_deref().and_then(parse_f64)).reduce(f64::max));
    let format_start_time = format.start_time.as_deref().and_then(parse_f64).unwrap_or(0.0);
    Ok(MediaInfo {
        path: path.to_string_lossy().into_owned(),
        format_name: format.format_name,
        format_start_time,
        duration_seconds,
        size_bytes: format.size.and_then(|v| v.parse().ok()),
        bit_rate: format.bit_rate.and_then(|v| v.parse().ok()),
        video,
        audio_streams,
    })
}

fn video_rate(video: &VideoInfo) -> Option<Rate> {
    parse_rate(video.average_frame_rate.as_deref())
        .or_else(|| parse_rate(video.real_frame_rate.as_deref()))
        .filter(|rate| rate.num > 0 && rate.den > 0)
}

fn parse_rate(text: Option<&str>) -> Option<Rate> {
    let (num, den) = text?.split_once('/')?;
    let num = num.parse::<u32>().ok()?;
    let den = den.parse::<u32>().ok()?;
    Rate::new(num, den).ok()
}

fn parse_f64(text: &str) -> Option<f64> {
    let value = text.parse::<f64>().ok()?;
    value.is_finite().then_some(value)
}

fn rate_eq(a: Rate, b: Rate) -> bool {
    u128::from(a.num) * u128::from(b.den) == u128::from(b.num) * u128::from(a.den)
}

fn rate_lt(a: Rate, b: Rate) -> bool {
    u128::from(a.num) * u128::from(b.den) < u128::from(b.num) * u128::from(a.den)
}

fn is_cfr(video: &VideoInfo) -> bool {
    parse_rate(video.average_frame_rate.as_deref())
        .zip(parse_rate(video.real_frame_rate.as_deref()))
        .is_some_and(|(avg, real)| rate_eq(avg, real))
}

fn is_hdr(video: &VideoInfo) -> bool {
    matches!(video.color_transfer.as_deref(), Some("smpte2084" | "arib-std-b67"))
}

fn video_filters(video: &VideoInfo, hdr: bool) -> Result<Vec<String>, String> {
    let mut filters = Vec::new();
    let sar = video.sample_aspect_ratio.as_deref().and_then(parse_ratio).unwrap_or((1, 1));
    if sar != (1, 1) {
        filters.push("scale=round(iw*sar/2)*2:ih,setsar=1,pad=ceil(iw/2)*2:ceil(ih/2)*2".into());
    } else if video.width.is_some_and(|v| v % 2 != 0) || video.height.is_some_and(|v| v % 2 != 0) {
        filters.push("pad=ceil(iw/2)*2:ceil(ih/2)*2".to_owned());
    }
    if hdr {
        filters.push("zscale=t=linear:npl=100,format=gbrpf32le,tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:p=bt709:r=tv,format=yuv420p".to_owned());
    }
    Ok(filters)
}

fn parse_ratio(value: &str) -> Option<(u32, u32)> {
    let (a, b) = value.split_once(':').or_else(|| value.split_once('/'))?;
    let a = a.parse().ok()?;
    let b = b.parse().ok()?;
    (a > 0 && b > 0).then_some((a, b))
}

fn copy_eligible(info: &MediaInfo, video: &VideoInfo) -> bool {
    let format_ok = info.format_name.as_deref().is_some_and(|format| {
        format.split(',').any(|name| matches!(name, "mov" | "mp4" | "m4a" | "3gp" | "3g2" | "mj2"))
    });
    let audio_ok = info.audio_streams.iter().all(|audio| audio.codec_name.as_deref() == Some("aac"));
    format_ok
        && video.codec_name.as_deref().is_some_and(|codec| matches!(codec, "h264" | "avc1"))
        && video.pixel_format.as_deref() == Some("yuv420p")
        && video.width.is_some_and(|width| width % 2 == 0)
        && video.height.is_some_and(|height| height % 2 == 0)
        && video.sample_aspect_ratio.as_deref().is_none_or(|sar| sar == "1:1")
        && video.rotation_degrees.is_none_or(|rotation| rotation.abs() < 0.1)
        && audio_ok
}


fn rounded_duration(duration: f64, rate: Rate) -> Result<f64, String> {
    let frames = (duration * f64::from(rate.num) / f64::from(rate.den)).ceil();
    if !frames.is_finite() || frames < 1.0 || frames > u64::MAX as f64 {
        return Err("目标帧数超出支持范围".into());
    }
    Ok(frames * f64::from(rate.den) / f64::from(rate.num))
}

fn rate_string(rate: Rate) -> String {
    format!("{}/{}", rate.num, rate.den)
}

fn format_seconds(seconds: f64) -> String {
    format!("{seconds:.9}")
}

fn encode_settings(quality: Quality) -> Vec<OsString> {
    let crf = match quality { Quality::Balanced => "20", Quality::High => "17" };
    vec![OsString::from("-preset"), OsString::from("medium"), OsString::from("-crf"), OsString::from(crf)]
}

fn progress_args() -> Vec<OsString> {
    vec![OsString::from("-progress"), OsString::from("pipe:1"), OsString::from("-nostats")]
}

fn ffmpeg_args<const N: usize>(args: [OsString; N]) -> Vec<OsString> {
    Vec::from(args)
}

fn run_ffmpeg(
    tools: &Tools,
    args: &[OsString],
    cancel: &Arc<AtomicBool>,
    verbose: bool,
    duration: Option<f64>,
) -> Result<(), String> {
    let mut full = Vec::with_capacity(args.len() + 2);
    full.push(OsString::from("-hide_banner"));
    full.push(OsString::from("-nostdin"));
    full.extend(args.iter().cloned());
    if !full.iter().any(|arg| arg == "-progress") {
        full.extend(progress_args());
    }
    process::run(&tools.ffmpeg, &full, cancel, verbose, duration)
}

fn mux_generated_video(
    tools: &Tools,
    generated_video: &Path,
    output: &Path,
    source: &MediaInfo,
    output_duration: f64,
    options: &ConvertOptions,
    cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    let video = source.video.as_ref().ok_or("源文件缺少视频流")?;
    let video_start = video.start_time.unwrap_or(source.format_start_time);
    let mut args = vec![
        OsString::from("-copyts"),
        OsString::from("-i"), generated_video.as_os_str().to_owned(),
        OsString::from("-itsoffset"), OsString::from(format_seconds(-video_start)),
    ];
    args.extend([
        OsString::from("-i"), OsString::from(source.path.as_str()),
        OsString::from("-map"), OsString::from("0:v:0"),
        OsString::from("-map"), OsString::from("1:a?"),
        OsString::from("-c:v"), OsString::from("copy"),
        OsString::from("-c:a"), OsString::from("aac"),
        OsString::from("-b:a"), OsString::from("192k"),
        OsString::from("-ar"), OsString::from("48000"),
        OsString::from("-t"), OsString::from(format_seconds(output_duration)),
        OsString::from("-video_track_timescale"), OsString::from(options.fps.num.to_string()),
        OsString::from("-map_metadata"), OsString::from("1"),
        OsString::from("-movflags"), OsString::from("+faststart"),
        OsString::from("-y"), output.as_os_str().to_owned(),
    ]);
    args.extend(progress_args());
    run_ffmpeg(tools, &args, cancel, options.verbose, Some(output_duration))
}

fn absolute_target(path: &Path) -> Result<PathBuf, String> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let parent = parent.canonicalize().map_err(|e| format!("无法访问目标目录 {}：{e}", parent.display()))?;
    let name = path.file_name().ok_or_else(|| format!("目标文件名无效：{}", path.display()))?;
    Ok(parent.join(name))
}

fn same_path(a: &Path, b: &Path) -> bool {
    if a == b { return true; }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn ensure_publish_allowed(path: &Path, overwrite: bool) -> Result<(), String> {
    if path.exists() && !overwrite {
        return Err(format!("目标文件已存在（如需替换请指定 --overwrite）：{}", path.display()));
    }
    Ok(())
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::SeqCst) { Err("操作已取消".into()) } else { Ok(()) }
}

struct TempWorkspace {
    path: PathBuf,
}

impl TempWorkspace {
    fn create(parent: &Path) -> Result<Self, String> {
        for _ in 0..32 {
            let path = unique_path(parent, ".smoothup-work");
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("无法创建临时工作目录 {}：{error}", path.display())),
            }
        }
        Err("无法分配唯一的临时工作目录".into())
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct StagedFile {
    path: PathBuf,
}

impl StagedFile {
    fn create(target: &Path, extension: &str) -> Result<Self, String> {
        let parent = target.parent().unwrap_or_else(|| Path::new("."));
        let stem = target.file_stem().and_then(|v| v.to_str()).unwrap_or("output");
        for _ in 0..32 {
            let path = unique_path(parent, &format!(".{stem}.smoothup-part"));
            let mut file_name = path.into_os_string();
            file_name.push(format!(".{extension}"));
            let path = PathBuf::from(file_name);
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    drop(file);
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("无法创建临时输出 {}：{error}", path.display())),
            }
        }
        Err("无法分配唯一的临时输出文件名".into())
    }

    fn publish(self, target: &Path, overwrite: bool) -> Result<(), String> {
        publish_file(&self.path, target, overwrite)
    }
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn write_report_temp(
    path: &Path,
    source: &MediaInfo,
    verification: &Verification,
    options: &ConvertOptions,
    engine: Engine,
    source_duration: f64,
    output_duration: f64,
) -> Result<StagedFile, String> {
    #[derive(Serialize)]
    struct Report<'a> {
        platform: Platform,
        input: &'a str,
        output: &'a str,
        target_frame_rate: Rate,
        engine: Engine,
        quality: Quality,
        source_duration_seconds: f64,
        output_duration_seconds: f64,
        source: &'a MediaInfo,
        verification: &'a Verification,
    }
    let report = Report {
        platform: options.platform,
        input: &source.path,
        output: &options.output.to_string_lossy(),
        target_frame_rate: options.fps,
        engine,
        quality: options.quality,
        source_duration_seconds: source_duration,
        output_duration_seconds: output_duration,
        source,
        verification,
    };
    let bytes = serde_json::to_vec_pretty(&report).map_err(|e| format!("无法序列化转换报告：{e}"))?;
    let staged = StagedFile::create(path, "json")?;
    let mut file = OpenOptions::new().write(true).truncate(true).open(&staged.path)
        .map_err(|e| format!("无法写入报告临时文件：{e}"))?;
    file.write_all(&bytes).map_err(|e| format!("无法写入报告临时文件：{e}"))?;
    file.sync_all().map_err(|e| format!("无法刷新报告临时文件：{e}"))?;
    Ok(staged)
}

fn unique_path(parent: &Path, prefix: &str) -> PathBuf {
    let time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO).as_nanos();
    let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    parent.join(format!("{prefix}-{}-{time:x}-{sequence:x}", std::process::id()))
}

fn publish_file(source: &Path, target: &Path, overwrite: bool) -> Result<(), String> {
    ensure_publish_allowed(target, overwrite)?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
        const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
        #[link(name = "Kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
        }
        let from: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        let flags = MOVEFILE_WRITE_THROUGH | if overwrite { MOVEFILE_REPLACE_EXISTING } else { 0 };
        let ok = unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), flags) };
        if ok == 0 {
            let error = std::io::Error::last_os_error();
            return Err(format!("无法安全发布 {}：{error}", target.display()));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        fs::rename(source, target).map_err(|e| format!("无法安全发布 {}：{e}", target.display()))
    }
}

