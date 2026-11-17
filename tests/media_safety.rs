use std::{fs, path::PathBuf, sync::{atomic::AtomicBool, Arc}};
use smoothup::{media, types::{ConvertOptions, Engine, Platform, Quality, Rate, Tools}};

struct Fixture { root: PathBuf, source: PathBuf }
impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("smoothup-media-{name}-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let source = root.join("source.mp4");
        fs::write(&source, b"source-must-survive").unwrap();
        Self { root, source }
    }
    fn options(&self) -> ConvertOptions {
        ConvertOptions { input: self.source.clone(), output: self.root.join("output.mp4"), platform: Platform::Bilibili, fps: Rate::new(60, 1).unwrap(), engine: Engine::Cpu, quality: Quality::Balanced, preview: None, report: None, verbose: false, overwrite: false }
    }
    fn tools(&self) -> Tools {
        Tools { ffmpeg: self.root.join("unavailable-ffmpeg"), ffprobe: self.root.join("unavailable-ffprobe"), rife: None, rife_model: None }
    }
    fn reject(&self, options: &ConvertOptions, message: &str) {
        let error = media::convert(&self.tools(), options, &Arc::new(AtomicBool::new(false))).unwrap_err();
        assert!(error.contains(message), "{error}");
        assert_eq!(fs::read(&self.source).unwrap(), b"source-must-survive");
    }
}
impl Drop for Fixture { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.root); } }

#[test]
fn report_must_not_replace_source_even_with_overwrite() {
    let fixture = Fixture::new("report-source");
    let mut options = fixture.options();
    options.report = Some(fixture.source.clone());
    options.overwrite = true;
    fixture.reject(&options, "报告路径");
    assert!(!options.output.exists());
}

#[test]
fn report_must_not_replace_video_output() {
    let fixture = Fixture::new("report-output");
    let mut options = fixture.options();
    options.report = Some(options.output.clone());
    fixture.reject(&options, "报告路径");
    assert!(!options.output.exists());
}

#[test]
fn existing_report_blocks_conversion_without_touching_files() {
    let fixture = Fixture::new("report-existing");
    let mut options = fixture.options();
    let report = fixture.root.join("existing.json");
    fs::write(&report, b"existing-report").unwrap();
    options.report = Some(report.clone());
    fixture.reject(&options, "已存在");
    assert_eq!(fs::read(report).unwrap(), b"existing-report");
    assert!(!options.output.exists());
}
