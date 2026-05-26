#![deny(warnings)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::module_name_repetitions,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    clippy::return_self_not_must_use
)]

pub mod config;
pub mod confidence;
pub mod exec_check;
pub mod executor;
pub mod hook;
pub mod journal;
pub mod pacman;
pub mod review;
pub mod scanners;
pub mod user_homes;
pub mod util;
