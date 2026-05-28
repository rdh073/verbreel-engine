//! Verbreel — CLI binary entrypoint.
//!
//! All routing lives in `verbreel_cli::run`. This binary is a thin
//! wrapper so integration tests can call the library function directly
//! without spawning subprocesses.

use std::io::{self, Write as _};

use clap::Parser;
use verbreel_cli::{Cli, run};

fn main() {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    let mut stdout = io::stdout().lock();
    let code = match run(cli, &mut stdout) {
        Ok(c) => c,
        Err(e) => {
            let _ = writeln!(io::stderr(), "error: {e}");
            1
        }
    };
    let _ = stdout.flush();
    std::process::exit(code);
}
