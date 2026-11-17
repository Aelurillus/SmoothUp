use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use crate::{process, Result, types::Tools};

const SETUP_SCRIPT: &str = include_str!("../scripts/setup.ps1");

pub fn discover() -> Result<Tools> {
    let managed = managed_dependency_root()?;
    let ffmpeg = discover_file("SMOOTHUP_FFMPEG", &managed.join("ffmpeg"), executable_name("ffmpeg"))?;
    let ffprobe = discover_file("SMOOTHUP_FFPROBE", &managed.join("ffmpeg"), executable_name("ffprobe"))?;
    let rife = discover_optional_file("SMOOTHUP_RIFE", &managed.join("rife"), executable_name("rife-ncnn-vulkan"))?;
    let rife_model = discover_optional_model("SMOOTHUP_RIFE_MODEL", &managed.join("rife"))?;

    Ok(Tools { ffmpeg, ffprobe, rife, rife_model })
}

pub fn doctor(verbose: bool, cancel: &Arc<AtomicBool>) -> Result<()> {
    println!("SmoothUp 环境诊断");
    let tools = match discover() {
        Ok(tools) => tools,
        Err(error) => {
            eprintln!("依赖发现失败：{error}");
            return Err(error);
        }
    };
    println!("FFmpeg：{}", tools.ffmpeg.display());
    check_version("FFmpeg", &tools.ffmpeg, cancel)?;
    println!("FFprobe：{}", tools.ffprobe.display());
    check_version("FFprobe", &tools.ffprobe, cancel)?;

    let encoders = process::capture(
        &tools.ffmpeg,
        &[OsString::from("-hide_banner"), OsString::from("-encoders")],
        cancel,
        4 * 1024 * 1024,
    )?;
    let encoder_list = String::from_utf8_lossy(&encoders);
    let has_x264 = encoder_list.lines().any(|line| line.split_whitespace().any(|word| word == "libx264"));
    let has_aac = encoder_list.lines().any(|line| line.split_whitespace().any(|word| word == "aac"));
    println!("H.264 libx264：{}", if has_x264 { "可用" } else { "不可用" });
    println!("AAC：{}", if has_aac { "可用" } else { "不可用" });
    if !has_x264 || !has_aac {
        return Err("当前 FFmpeg 缺少 SmoothUp 所需的 libx264 或 AAC 编码器".into());
    }

    match (&tools.rife, &tools.rife_model) {
        (Some(exe), Some(model)) => {
            println!("RIFE Vulkan：{}", exe.display());
            println!("RIFE v4.6 模型：{}", model.display());
            println!("已定位 RIFE 可执行文件及 v4.6 模型；GPU/Vulkan 尚未实测，请用短预览确认可用性。");
        }
        (Some(exe), None) => println!("RIFE 引擎：{}（未找到 v4.6 模型；CPU 插帧仍可用）", exe.display()),
        (None, Some(model)) => println!("RIFE 模型：{}（未找到 RIFE 引擎；CPU 插帧仍可用）", model.display()),
        (None, None) => println!("RIFE：未安装（CPU 插帧可用）"),
    }
    if verbose {
        println!("管理目录：{}", managed_home()?.display());
        println!("独立 RIFE 仅在显式选择或自动选择时使用，不会自动联网下载依赖。");
    }
    println!("FFmpeg / H.264 / AAC 检查通过。");
    Ok(())
}

pub fn setup(rife: bool, verbose: bool, cancel: &Arc<AtomicBool>) -> Result<()> {
    if cancel.load(Ordering::SeqCst) {
        return Err("操作已取消".into());
    }
    let home = managed_home()?;
    fs::create_dir_all(&home).map_err(|error| format!("无法创建管理目录 {}：{error}", home.display()))?;
    let script_dir = home.join("cache");
    fs::create_dir_all(&script_dir).map_err(|error| format!("无法创建安装脚本目录 {}：{error}", script_dir.display()))?;
    let script = script_dir.join("setup.ps1");
    let script_tmp = script_dir.join(format!("setup.{}.partial", std::process::id()));
    fs::write(&script_tmp, SETUP_SCRIPT).map_err(|error| format!("无法写入安装脚本：{error}"))?;
    if script.exists() {
        fs::remove_file(&script).map_err(|error| format!("无法更新安装脚本：{error}"))?;
    }
    fs::rename(&script_tmp, &script).map_err(|error| format!("无法安装脚本：{error}"))?;

    let mut args = vec![
        OsString::from("-NoLogo"),
        OsString::from("-NoProfile"),
        OsString::from("-NonInteractive"),
        OsString::from("-ExecutionPolicy"),
        OsString::from("Bypass"),
        OsString::from("-File"),
        script.into_os_string(),
    ];
    if rife { args.push(OsString::from("-Rife")); }
    if verbose { args.push(OsString::from("-VerboseOutput")); }
    let powershell = find_powershell()?;
    println!("正在安装固定版本的 FFmpeg{}……", if rife { " 与 RIFE v4.6" } else { "" });
    process::run(&powershell, &args, cancel, verbose, None)?;
    println!("安装完成：{}", managed_dependency_root()?.display());
    println!("建议运行 smoothup doctor 检查 FFmpeg 与编码器环境。\nRIFE 的 GPU/Vulkan 支持请用短预览实际确认。");
    Ok(())
}

fn check_version(label: &str, program: &Path, cancel: &Arc<AtomicBool>) -> Result<()> {
    let output = process::capture(
        program,
        &[OsString::from("-version")],
        cancel,
        128 * 1024,
    )?;
    let text = String::from_utf8_lossy(&output);
    let version = text.lines().next().unwrap_or("版本信息不可用");
    println!("{label}：{version}");
    Ok(())
}

fn discover_file(env_name: &str, managed_dir: &Path, name: OsString) -> Result<PathBuf> {
    if let Some(value) = env::var_os(env_name) {
        return validate_explicit(env_name, PathBuf::from(value));
    }
    let managed = managed_dir.join(&name);
    if managed.is_file() {
        return Ok(managed);
    }
    find_in_path(&name).ok_or_else(|| format!("找不到 {}；请先运行 smoothup setup，或设置 {env_name}", name.to_string_lossy()))
}

fn discover_optional_file(env_name: &str, managed_dir: &Path, name: OsString) -> Result<Option<PathBuf>> {
    if let Some(value) = env::var_os(env_name) {
        return validate_explicit(env_name, PathBuf::from(value)).map(Some);
    }
    let managed = managed_dir.join(&name);
    if managed.is_file() {
        return Ok(Some(managed));
    }
    Ok(find_in_path(&name))
}

fn discover_optional_model(env_name: &str, managed_rife_dir: &Path) -> Result<Option<PathBuf>> {
    if let Some(value) = env::var_os(env_name) {
        let path = PathBuf::from(value);
        if is_rife_model(&path) {
            return Ok(Some(path));
        }
        return Err(format!("{env_name} 指向无效的 RIFE 模型目录：{}", path.display()));
    }
    let model = managed_rife_dir.join("rife-v4.6");
    if is_rife_model(&model) { Ok(Some(model)) } else { Ok(None) }
}

fn is_rife_model(path: &Path) -> bool {
    path.join("flownet.bin").is_file() && path.join("flownet.param").is_file()
}

fn validate_explicit(env_name: &str, path: PathBuf) -> Result<PathBuf> {
    if !path.is_file() {
        return Err(format!("{env_name} 指向不存在的可执行文件：{}", path.display()));
    }
    Ok(path)
}

fn managed_home() -> Result<PathBuf> {
    if let Some(value) = env::var_os("SMOOTHUP_HOME") {
        if value.is_empty() {
            return Err("SMOOTHUP_HOME 不能为空".into());
        }
        return Ok(PathBuf::from(value));
    }
    if let Some(value) = env::var_os("LOCALAPPDATA") {
        return Ok(PathBuf::from(value).join("SmoothUp"));
    }
    if cfg!(windows) {
        return Err("LOCALAPPDATA 未设置；请设置 SMOOTHUP_HOME 指定管理目录".into());
    }
    let home = env::var_os("HOME").ok_or_else(|| "HOME 未设置；请设置 SMOOTHUP_HOME".to_owned())?;
    Ok(PathBuf::from(home).join(".smoothup"))
}

fn managed_dependency_root() -> Result<PathBuf> {
    Ok(managed_home()?.join("deps"))
}

fn executable_name(stem: &str) -> OsString {
    if cfg!(windows) { OsString::from(format!("{stem}.exe")) } else { OsString::from(stem) }
}

fn find_in_path(name: &std::ffi::OsStr) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    for dir in env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        if Path::new(name).extension().is_none() {
            let candidate = dir.join(format!("{}.exe", name.to_string_lossy()));
            if candidate.is_file() { return Some(candidate); }
        }
    }
    None
}

fn find_powershell() -> Result<PathBuf> {
    for name in ["pwsh.exe", "powershell.exe", "pwsh", "powershell"] {
        if let Some(path) = find_in_path(std::ffi::OsStr::new(name)) {
            return Ok(path);
        }
    }
    Err("未找到 PowerShell；Windows 安装需要 pwsh 或 powershell".into())
}
