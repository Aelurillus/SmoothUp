use std::{
    ffi::OsString,
    fs,
    io::Write,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use crate::{process, types::{Rate, Tools}};

use super::{VideoInfo, encode_settings, ffmpeg_args, run_ffmpeg};

const SOURCE_CHUNK_FRAMES: u64 = 8;
const OUTPUT_CHUNK_FRAMES: u64 = 8;
const SCENE_SCAN_LIMIT: usize = 16 * 1024 * 1024;

pub(super) struct RenderRequest<'a> {
    pub input: &'a Path,
    pub destination: &'a Path,
    pub workspace: &'a Path,
    pub video: &'a VideoInfo,
    pub source_rate: Rate,
    pub target_rate: Rate,
    pub frame_count: u64,
    pub duration: f64,
    pub format_start_time: f64,
    pub filters: &'a str,
    pub quality: crate::types::Quality,
    pub verbose: bool,
}

struct RunContext<'a> {
    cancel: &'a Arc<AtomicBool>,
    verbose: bool,
}

pub(super) fn render(
    tools: &Tools,
    request: RenderRequest<'_>,
    cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    let RenderRequest {
        input, destination, workspace, video, source_rate, target_rate,
        frame_count, duration, format_start_time, filters, quality, verbose,
    } = request;
    let run = RunContext { cancel, verbose };
    let executable = tools.rife.as_ref().ok_or_else(|| "未找到 RIFE 可执行文件".to_owned())?;
    let model = tools.rife_model.as_ref().ok_or_else(|| "未找到 RIFE v4.6 模型".to_owned())?;
    if frame_count < 2 {
        return Err("RIFE 至少需要两帧视频输入".into());
    }
    if !source_rate_matches(video, source_rate) {
        return Err("RIFE 当前仅支持恒定帧率输入；请使用 --engine cpu 处理可变帧率视频".into());
    }
    if cancel.load(Ordering::SeqCst) {
        return Err("操作已取消".into());
    }

    eprintln!("正在检测场景切换…");
    let cuts = detect_scene_cuts(tools, input, video, source_rate, format_start_time, cancel)?;
    let source_dir = workspace.join("rife-source");
    let output_dir = workspace.join("rife-output");
    fs::create_dir_all(&source_dir).map_err(|e| format!("无法创建 RIFE 源帧目录：{e}"))?;
    fs::create_dir_all(&output_dir).map_err(|e| format!("无法创建 RIFE 输出帧目录：{e}"))?;

    let output_count = frame_count_for_duration(duration, target_rate)?;
    let playlist = workspace.join("rife-concat.txt");
    let mut playlist_file = fs::File::create(&playlist).map_err(|e| e.to_string())?;
    let mut chunk_index = 0_u64;
    let mut frames_in_chunk = 0_u64;
    let mut source_chunk_start = u64::MAX;

    for output_index in 0..output_count {
        if cancel.load(Ordering::SeqCst) {
            return Err("操作已取消".into());
        }
        let (source_index, fraction) = source_position(output_index, source_rate, target_rate);
        let source_index = source_index.min(frame_count.saturating_sub(1));
        let needs_next = source_index + 1 < frame_count && fraction > 0.000000001;
        let desired_start = (source_index / SOURCE_CHUNK_FRAMES) * SOURCE_CHUNK_FRAMES;
        if desired_start != source_chunk_start {
            if source_chunk_start != u64::MAX {
                fs::remove_dir_all(&source_dir).map_err(|e| format!("无法清理 RIFE 源帧分块：{e}"))?;
                fs::create_dir_all(&source_dir).map_err(|e| format!("无法创建 RIFE 源帧目录：{e}"))?;
            }
            source_chunk_start = desired_start;
            let source_chunk_end = (desired_start + SOURCE_CHUNK_FRAMES + 1).min(frame_count);
            extract_source_chunk(
                tools, input, &source_dir, source_chunk_start, source_chunk_end,
                filters, &run,
            )?;
        }

        let local_index = source_index - source_chunk_start;
        let first = source_dir.join(format!("frame{local_index:08}.png"));
        let mut selected = first.clone();
        if needs_next {
            let next_index = source_index + 1;
            let cut_at_next = cuts.binary_search(&next_index).is_ok();
            if cut_at_next {
                // Keep the pre-cut frame until the exact cut; never synthesize across it.
                selected = first.clone();
            } else {
                let second = source_dir.join(format!("frame{:08}.png", next_index - source_chunk_start));
                let output = output_dir.join(format!("frame{frames_in_chunk:08}.png"));
                interpolate_pair(executable, model, &first, &second, &output, fraction, &run)?;
                selected = output;
            }
        }

        if selected == first {
            let output = output_dir.join(format!("frame{frames_in_chunk:08}.png"));
            fs::copy(&first, &output).map_err(|e| format!("無法保存 RIFE 输出帧：{e}"))?;
        }
        frames_in_chunk += 1;

        if frames_in_chunk == OUTPUT_CHUNK_FRAMES || output_index + 1 == output_count {
            let chunk_path = workspace.join(format!("rife-chunk-{chunk_index:08}.nut"));
            encode_frame_chunk(
                tools, &output_dir, &chunk_path, frames_in_chunk, target_rate,
                quality, &run,
            )?;
            let chunk_name = chunk_path.to_string_lossy();
            let chunk_name = if let Some(unc) = chunk_name.strip_prefix(r"\\?\UNC\") {
                format!("//{unc}")
            } else {
                chunk_name.strip_prefix(r"\\?\").unwrap_or(&chunk_name).to_owned()
            };
            let chunk_name = chunk_name.replace('\\', "/").replace('\'', "'\\''");
            writeln!(playlist_file, "file 'file:{chunk_name}'").map_err(|e| e.to_string())?;
            chunk_index += 1;
            frames_in_chunk = 0;
            fs::remove_dir_all(&output_dir).map_err(|e| format!("无法清理 RIFE 输出帧分块：{e}"))?;
            fs::create_dir_all(&output_dir).map_err(|e| format!("无法创建 RIFE 输出帧目录：{e}"))?;
        }
    }

    drop(playlist_file);

    let args = ffmpeg_args([
        OsString::from("-f"), OsString::from("concat"),
        OsString::from("-safe"), OsString::from("0"),
        OsString::from("-i"), playlist.as_os_str().to_owned(),
        OsString::from("-map"), OsString::from("0:v:0"),
        OsString::from("-c:v"), OsString::from("copy"),
        OsString::from("-bsf:v"), OsString::from(format!(
            "setts=pts=N*{}:dts=N*{}:duration={}:time_base=1/{}",
            target_rate.den, target_rate.den, target_rate.den, target_rate.num,
        )),
        OsString::from("-video_track_timescale"), OsString::from(target_rate.num.to_string()),
        OsString::from("-movflags"), OsString::from("+faststart"),
        OsString::from("-an"), OsString::from("-y"), destination.as_os_str().to_owned(),
    ]);
    run_ffmpeg(tools, &args, cancel, verbose, Some(duration))?;
    Ok(())
}

fn source_rate_matches(video: &VideoInfo, rate: Rate) -> bool {
    let Some(real) = parse_rate(video.real_frame_rate.as_deref()) else { return false };
    real.num == rate.num && real.den == rate.den
}

fn parse_rate(value: Option<&str>) -> Option<Rate> {
    let (num, den) = value?.split_once('/')?;
    Rate::new(num.parse().ok()?, den.parse().ok()?).ok()
}

fn frame_count_for_duration(duration: f64, rate: Rate) -> Result<u64, String> {
    if !duration.is_finite() || duration <= 0.0 {
        return Err("视频时长无效".into());
    }
    let frames = (duration * f64::from(rate.num) / f64::from(rate.den)).ceil();
    if !frames.is_finite() || frames < 1.0 || frames > u64::MAX as f64 {
        return Err("目标帧数超出支持范围".into());
    }
    Ok(frames as u64)
}

fn source_position(output_index: u64, source_rate: Rate, target_rate: Rate) -> (u64, f64) {
    let numerator = u128::from(output_index)
        .saturating_mul(u128::from(target_rate.den))
        .saturating_mul(u128::from(source_rate.num));
    let denominator = u128::from(target_rate.num) * u128::from(source_rate.den);
    let index = numerator / denominator;
    let remainder = numerator % denominator;
    let fraction = remainder as f64 / denominator as f64;
    (index.min(u128::from(u64::MAX)) as u64, fraction)
}

fn extract_source_chunk(
    tools: &Tools,
    input: &Path,
    directory: &Path,
    start: u64,
    end: u64,
    filters: &str,
    run: &RunContext<'_>,
) -> Result<(), String> {
    if end <= start {
        return Err("RIFE 源帧分块范围无效".into());
    }
    let selection = format!("select='between(n,{start},{})'", end - 1);
    let filters = if filters.is_empty() { selection } else { format!("{selection},{filters}") };
    let pattern = directory.join("frame%08d.png");
    let count = end - start;
    let args = ffmpeg_args([
        OsString::from("-i"), input.as_os_str().to_owned(),
        OsString::from("-map"), OsString::from("0:v:0"),
        OsString::from("-vf"), OsString::from(filters),
        OsString::from("-frames:v"), OsString::from(count.to_string()),
        OsString::from("-fps_mode"), OsString::from("passthrough"),
        OsString::from("-start_number"), OsString::from("0"),
        OsString::from("-compression_level"), OsString::from("1"),
        OsString::from("-y"), pattern.as_os_str().to_owned(),
    ]);
    run_ffmpeg(tools, &args, run.cancel, run.verbose, None)?;
    let last = directory.join(format!("frame{:08}.png", count - 1));
    if !last.is_file() {
        return Err(format!("FFmpeg 未能提取 RIFE 源帧分块 {start}..{end}"));
    }
    Ok(())
}

fn interpolate_pair(
    executable: &Path,
    model: &Path,
    first: &Path,
    second: &Path,
    output: &Path,
    fraction: f64,
    run: &RunContext<'_>,
) -> Result<(), String> {
    if run.cancel.load(Ordering::SeqCst) {
        return Err("操作已取消".into());
    }
    let args = ffmpeg_args([
        OsString::from("-0"), first.as_os_str().to_owned(),
        OsString::from("-1"), second.as_os_str().to_owned(),
        OsString::from("-o"), output.as_os_str().to_owned(),
        OsString::from("-s"), OsString::from(format!("{fraction:.9}")),
        OsString::from("-m"), model.as_os_str().to_owned(),
    ]);
    process::run(executable, &args, run.cancel, run.verbose, None)
}

fn encode_frame_chunk(
    tools: &Tools,
    images: &Path,
    destination: &Path,
    frame_count: u64,
    rate: Rate,
    quality: crate::types::Quality,
    run: &RunContext<'_>,
) -> Result<(), String> {
    let pattern = images.join("frame%08d.png");
    let mut args = vec![
        OsString::from("-framerate"), OsString::from(format!("{}/{}", rate.num, rate.den)),
        OsString::from("-start_number"), OsString::from("0"),
        OsString::from("-i"), pattern.as_os_str().to_owned(),
        OsString::from("-frames:v"), OsString::from(frame_count.to_string()),
        OsString::from("-an"), OsString::from("-c:v"), OsString::from("libx264"),
        OsString::from("-bf"), OsString::from("0"),
    ];
    args.extend(encode_settings(quality));
    args.extend([
        OsString::from("-pix_fmt"), OsString::from("yuv420p"),
        OsString::from("-fps_mode"), OsString::from("cfr"),
        OsString::from("-enc_time_base"), OsString::from(format!("{}/{}", rate.den, rate.num)),
        OsString::from("-color_primaries"), OsString::from("bt709"),
        OsString::from("-color_trc"), OsString::from("bt709"),
        OsString::from("-colorspace"), OsString::from("bt709"),
        OsString::from("-y"), destination.as_os_str().to_owned(),
    ]);
    run_ffmpeg(tools, &args, run.cancel, run.verbose, None)
}

fn detect_scene_cuts(
    tools: &Tools,
    input: &Path,
    video: &VideoInfo,
    source_rate: Rate,
    format_start_time: f64,
    cancel: &Arc<AtomicBool>,
) -> Result<Vec<u64>, String> {
    let args = ffmpeg_args([
        OsString::from("-i"), input.as_os_str().to_owned(),
        OsString::from("-hide_banner"), OsString::from("-nostdin"),
        OsString::from("-map"), OsString::from("0:v:0"),
        OsString::from("-vf"), OsString::from("select='gt(scene,0.30)'"),
        OsString::from("-fps_mode"), OsString::from("passthrough"),
        OsString::from("-an"), OsString::from("-f"), OsString::from("framehash"),
        OsString::from("-hash"), OsString::from("md5"), OsString::from("pipe:1"),
    ]);
    let bytes = process::capture(&tools.ffmpeg, &args, cancel, SCENE_SCAN_LIMIT)?;
    let text = String::from_utf8(bytes).map_err(|e| format!("FFmpeg 场景检测输出编码无效：{e}"))?;
    let time_base = text.lines().find_map(|line| {
        let value = line.strip_prefix("#tb 0: ")?;
        let (num, den) = value.split_once('/')?;
        Some((num.parse::<f64>().ok()?, den.parse::<f64>().ok()?))
    });
    let Some((tb_num, tb_den)) = time_base else { return Ok(Vec::new()) };
    if tb_num <= 0.0 || tb_den <= 0.0 { return Err("FFmpeg 场景检测时间基准无效".into()); }
    let video_start = video.start_time.unwrap_or(format_start_time) - format_start_time;
    let fps = f64::from(source_rate.num) / f64::from(source_rate.den);
    let mut cuts = Vec::new();
    for line in text.lines().filter(|line| !line.starts_with('#')) {
        let Some(pts) = line.split(',').nth(2).and_then(|v| v.trim().parse::<i64>().ok()) else { continue };
        let time = pts as f64 * tb_num / tb_den;
        let index = ((time - video_start) * fps).round();
        if index.is_finite() && index >= 1.0 && index <= u64::MAX as f64 {
            cuts.push(index as u64);
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    Ok(cuts)
}

