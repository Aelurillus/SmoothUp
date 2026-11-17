pub mod media;
pub mod process;
pub mod tools;
pub mod types;

pub type Result<T, E = String> = std::result::Result<T, E>;
