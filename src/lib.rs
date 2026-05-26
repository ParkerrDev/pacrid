// Safety-critical lint policy. The goal is to make whole classes of bugs
// impossible to merge — every lint here is enforced as an error in CI.
//
// We intentionally take a stricter line than the Rust default: lints from
// `clippy::pedantic` are warnings, and a hand-picked set of lints from
// `clippy::restriction` (which is otherwise too noisy for general use) are
// promoted to deny. Each `allow` below is a deliberate, justified exception.
#![deny(warnings)]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::panic)]
#![deny(clippy::indexing_slicing)]
#![deny(clippy::arithmetic_side_effects)]
#![deny(clippy::float_arithmetic)]
#![deny(clippy::dbg_macro)]
#![deny(clippy::todo)]
#![deny(clippy::unimplemented)]
#![deny(clippy::lossy_float_literal)]
#![deny(clippy::mem_forget)]
#![deny(clippy::exit)]
#![deny(clippy::str_to_string)]
#![deny(clippy::large_stack_arrays)]
#![deny(clippy::large_stack_frames)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::module_name_repetitions,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    clippy::return_self_not_must_use
)]

pub mod confidence;
pub mod config;
pub mod exec_check;
pub mod executor;
pub mod hook;
pub mod journal;
pub mod pacman;
pub mod review;
pub mod scanners;
pub mod user_homes;
pub mod util;
