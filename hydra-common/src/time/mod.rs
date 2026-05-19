//! Shared time helpers — currently the CLI time-window parser used by
//! `hydra graph diff` (PR 4) and `hydra graph log` (PR 5).

pub mod parse;

pub use parse::{HydraTime, TimeParseError, parse_window_arg};
