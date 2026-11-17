use std::{fmt, path::PathBuf, str::FromStr};

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::Result;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Rate {
    pub num: u32,
    pub den: u32,
}

impl Rate {
    pub fn new(num: u32, den: u32) -> Result<Self> {
        if num == 0 || den == 0 {
            return Err("帧率的分子与分母必须大于零".into());
        }
        let mut a = num;
        let mut b = den;
        while b != 0 {
            (a, b) = (b, a % b);
        }
        Ok(Self { num: num / a, den: den / a })
    }

    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }
}

impl FromStr for Rate {
    type Err = String;

    fn from_str(value: &str) -> Result<Self> {
        let invalid = || format!("无效帧率 {value:?}；请使用 60、120、60000/1001 等正有理数");
        if let Some((num, den)) = value.split_once('/') {
            Self::new(num.parse().map_err(|_| invalid())?, den.parse().map_err(|_| invalid())?)
        } else {
            Self::new(value.parse().map_err(|_| invalid())?, 1)
        }
    }
}

impl fmt::Display for Rate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 { write!(f, "{}", self.num) } else { write!(f, "{}/{}", self.num, self.den) }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Platform { Douyin, Bilibili }

impl Platform {
    pub fn as_str(self) -> &'static str {
        match self { Self::Douyin => "douyin", Self::Bilibili => "bilibili" }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Engine { Auto, Cpu, Rife }

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Quality { Balanced, High }

#[derive(Clone, Debug)]
pub struct ConvertOptions {
    pub input: PathBuf,
    pub output: PathBuf,
    pub platform: Platform,
    pub fps: Rate,
    pub engine: Engine,
    pub quality: Quality,
    pub preview: Option<f64>,
    pub report: Option<PathBuf>,
    pub verbose: bool,
    pub overwrite: bool,
}

#[derive(Clone, Debug)]
pub struct Tools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub rife: Option<PathBuf>,
    pub rife_model: Option<PathBuf>,
}
