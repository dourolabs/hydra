//! Shared time-handling utilities for the Hydra workspace.
//!
//! Currently exposes [`parse_window_arg`], the single parser for the
//! `--since` / `--until` flags on `hydra graph diff` / `hydra graph log`.

pub mod parse;

pub use parse::{TimeParseError, parse_window_arg, parse_window_arg_with_now};
