//! verbreel-conformance — binary entrypoint.
//!
//! All logic lives in `verbreel_conformance::run`. This binary is a
//! thin wrapper so integration tests can call the library function
//! directly without spawning subprocesses.

use std::io::{self, Write as _};
use std::process;

fn main() {
    let mut stdout = io::stdout().lock();
    let code = verbreel_conformance::run(&mut stdout);
    let _ = stdout.flush();
    process::exit(code);
}
