use std::{
    path::{Path, PathBuf},
    process::ExitCode,
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use clap::{Args, Parser, Subcommand};
use smoothup::{
    media::{self, MediaInfo, Verification},
    tools,
    types::{ConvertOptions, Engine, Platform, Quality, Rate},
};

#[derive(Parser)]
#[command(
    name = "smoothup",
    version,
    about = "轻量、可靠的视频高帧率处理工具",
    long_about = "SmoothUp：为抖音与哔哩哔哩准备高帧率视频。不会在启动或转换时自动联网。",
    subcommand_required = false,
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    convert: ConvertArgs,
}

#[derive(Args)]
struct ConvertArgs {
    #[arg(value_name = "输入视频", help = "要处理的视频文件")]
    input: Option<PathBuf>,

    #[arg(long, value_enum, help = "目标平台：douyin 或 bilibili")]
    platform: Option<Platform>,

    #[arg(long, value_parser = parse_rate, default_value = "60", value_name = "帧率", help = "目标帧率：60、120、60000/1001 或 120000/1001")]
    fps: Rate,

    #[arg(long, value_name = "文件", help = "输出文件；默认使用 输入名.平台.帧率fps.mp4")]
    output: Option<PathBuf>,

    #[arg(long, value_enum, default_value = "auto", help = "插帧引擎：auto、cpu 或 rife")]
    engine: Engine,

    #[arg(long, value_enum, default_value = "balanced", help = "处理质量：balanced 或 high")]
    quality: Quality,

    #[arg(long, value_parser = parse_positive_seconds, value_name = "秒", help = "仅处理开头指定秒数，用于预览")]
    preview: Option<f64>,

    #[arg(long, value_name = "文件", help = "将处理报告写入 JSON 文件")]
    report: Option<PathBuf>,

    #[arg(long, help = "显示详细诊断信息")]
    verbose: bool,

    #[arg(long, help = "允许替换已有输出文件")]
    overwrite: bool,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "检查 FFmpeg、编码器及可选 RIFE 环境")]
    Doctor {
        #[arg(long, help = "显示详细诊断信息")]
        verbose: bool,
    },
    #[command(about = "安装固定版本的 FFmpeg；可选安装 RIFE v4.6")]
    Setup {
        #[arg(long, help = "同时安装 RIFE Vulkan 引擎及唯一的 v4.6 模型")]
        rife: bool,
        #[arg(long, help = "显示下载与安装细节")]
        verbose: bool,
    },
    #[command(about = "读取视频流信息")]
    Inspect {
        #[arg(value_name = "文件")]
        file: PathBuf,
        #[arg(long, help = "以 JSON 输出")]
        json: bool,
    },
    #[command(about = "完整解码并检查视频时序")]
    Verify {
        #[arg(value_name = "文件")]
        file: PathBuf,
        #[arg(long, value_parser = parse_rate, value_name = "帧率", help = "可选的预期目标帧率")]
        fps: Option<Rate>,
        #[arg(long, help = "以 JSON 输出")]
        json: bool,
        #[arg(long, help = "显示详细检查信息")]
        verbose: bool,
    },
}

fn parse_rate(value: &str) -> Result<Rate, String> {
    let rate = Rate::from_str(value)?;
    let allowed = [
        Rate::new(60, 1)?,
        Rate::new(120, 1)?,
        Rate::new(60_000, 1_001)?,
        Rate::new(120_000, 1_001)?,
    ];
    if allowed.contains(&rate) {
        Ok(rate)
    } else {
        Err("目标帧率仅支持 60、120、60000/1001 或 120000/1001".into())
    }
}

fn parse_positive_seconds(value: &str) -> Result<f64, String> {
    let seconds: f64 = value
        .parse()
        .map_err(|_| "预览时长必须是正数（秒）".to_owned())?;
    if seconds.is_finite() && seconds > 0.0 {
        Ok(seconds)
    } else {
        Err("预览时长必须是有限的正数（秒）".into())
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let cancelled = Arc::new(AtomicBool::new(false));
    let signal_flag = Arc::clone(&cancelled);
    if let Err(error) = ctrlc::set_handler(move || {
        signal_flag.store(true, Ordering::SeqCst);
    }) {
        eprintln!("无法注册取消处理器：{error}");
        return ExitCode::FAILURE;
    }

    match execute(cli, &cancelled) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if cancelled.load(Ordering::SeqCst) {
                eprintln!("已取消：{error}");
                ExitCode::from(130)
            } else {
                eprintln!("错误：{error}");
                ExitCode::FAILURE
            }
        }
    }
}

fn execute(cli: Cli, cancel: &Arc<AtomicBool>) -> smoothup::Result<()> {
    match cli.command {
        Some(Command::Doctor { verbose }) => tools::doctor(verbose, cancel),
        Some(Command::Setup { rife, verbose }) => tools::setup(rife, verbose, cancel),
        Some(Command::Inspect { file, json }) => {
            let discovered = tools::discover()?;
            let info = media::inspect(&discovered, &file, cancel)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&info).map_err(|e| e.to_string())?);
            } else {
                print_media_info(&info);
            }
            Ok(())
        }
        Some(Command::Verify { file, fps, json, verbose }) => {
            let discovered = tools::discover()?;
            let result = media::verify(&discovered, &file, fps, cancel, verbose)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?);
            } else {
                print_verification(&result);
            }
            if result.valid { Ok(()) } else { Err("验证未通过；请查看上述错误信息".into()) }
        }
        None => convert(cli.convert, cancel),
    }
}

fn convert(args: ConvertArgs, cancel: &Arc<AtomicBool>) -> smoothup::Result<()> {
    let input = args.input.ok_or_else(|| "请提供输入视频文件".to_owned())?;
    let platform = args.platform.ok_or_else(|| "转换时必须指定 --platform douyin 或 --platform bilibili".to_owned())?;
    let output = args.output.unwrap_or_else(|| default_output(&input, platform, args.fps, args.preview.is_some()));
    let options = ConvertOptions {
        input,
        output,
        platform,
        fps: args.fps,
        engine: args.engine,
        quality: args.quality,
        preview: args.preview,
        report: args.report,
        verbose: args.verbose,
        overwrite: args.overwrite,
    };
    media::convert(&tools::discover()?, &options, cancel).map(|_| ())
}

fn default_output(input: &Path, platform: Platform, rate: Rate, preview: bool) -> PathBuf {
    let stem = input.file_stem().unwrap_or_default().to_string_lossy();
    let fps = if rate.den == 1 {
        rate.num.to_string()
    } else {
        format!("{}-{}", rate.num, rate.den)
    };
    let preview_suffix = if preview { ".preview" } else { "" };
    let name = format!("{stem}.{}.{}fps{preview_suffix}.mp4", platform.as_str(), fps);
    input.parent().unwrap_or_else(|| Path::new("")).join(name)
}

fn print_media_info(info: &MediaInfo) {
    println!("媒体信息：{}", info.path);
    if let Some(format) = &info.format_name { println!("容器：{format}"); }
    if let Some(duration) = info.duration_seconds { println!("时长：{duration:.3} 秒"); }
    if let Some(size) = info.size_bytes { println!("文件大小：{size} 字节"); }
    if let Some(video) = &info.video {
        println!("视频：");
        if let Some(codec) = &video.codec_name { println!("  编码：{codec}"); }
        if let (Some(width), Some(height)) = (video.width, video.height) { println!("  尺寸：{width} × {height}"); }
        if let Some(pixel) = &video.pixel_format { println!("  像素格式：{pixel}"); }
        if let Some(rate) = &video.average_frame_rate { println!("  平均帧率：{rate}"); }
        if let Some(frames) = video.frame_count { println!("  帧数：{frames}"); }
        if let Some(rotation) = video.rotation_degrees { if rotation != 0.0 { println!("  旋转：{rotation}°"); } }
    }
    for (index, audio) in info.audio_streams.iter().enumerate() {
        let codec = audio.codec_name.as_deref().unwrap_or("未知编码");
        let channels = audio.channels.map(|count| format!("，{count} 声道")).unwrap_or_default();
        let sample_rate = audio.sample_rate.as_deref().map(|rate| format!("，{rate} Hz")).unwrap_or_default();
        println!("音轨 {}：{codec}{channels}{sample_rate}", index + 1);
    }
}

fn print_verification(result: &Verification) {
    println!("文件：{}", result.path);
    println!("验证结果：{}", if result.valid { "通过" } else { "未通过" });
    if let Some(rate) = result.expected_frame_rate { println!("目标帧率：{rate}"); }
    if let Some(rate) = result.actual_frame_rate { println!("实际帧率：{rate}"); }
    if let Some(duration) = result.duration_seconds { println!("时长：{duration:.3} 秒"); }
    if let Some(codec) = &result.video_codec { println!("视频编码：{codec}"); }
    if let Some(pixel) = &result.pixel_format { println!("像素格式：{pixel}"); }
    println!("完整解码：{}", if result.decoded_successfully { "成功" } else { "失败" });
    for error in &result.errors { println!("错误：{error}"); }
    for warning in &result.warnings { println!("提示：{warning}"); }
}
