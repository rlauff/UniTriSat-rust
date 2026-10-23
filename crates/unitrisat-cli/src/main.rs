//! The `unitrisat` binary: parses the command line and runs it
//! ([`unitrisat_cli::cli`]).
//!
//! Exit status: 0 on success, 1 if a polytope failed or the run could not
//! go on, 2 for a usage error.

#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::unreachable,
    clippy::disallowed_macros,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic_in_result_fn,
    clippy::assertions_on_constants
)]

use std::{
    io::{self, IsTerminal as _, Write as _},
    process::ExitCode,
};
use unitrisat_cli::cli::{Cli, execute_interactive};

fn main() -> ExitCode {
    let cli = match Cli::try_parse_input_from(std::env::args_os()) {
        Ok(cli) => cli,
        Err(error) => {
            // Help and version requests are not errors (exit code 0).
            let _ = error.print();
            return ExitCode::from(u8::try_from(error.exit_code()).unwrap_or(2));
        }
    };
    let mut out = io::stdout();
    let mut err = io::stderr();
    let out_terminal = out.is_terminal();
    let err_terminal = err.is_terminal();
    match execute_interactive(&cli, &mut out, &mut err, out_terminal, err_terminal) {
        Ok(tally) if tally.failed == 0 => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(error) => {
            let _ = writeln!(err, "error: {error}");
            ExitCode::FAILURE
        }
    }
}
