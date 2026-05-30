//! Python sidecar protocol — JSON over stdin/stdout, deterministic schema.
//!
//! Workloads that are not cleanly native yet (faster-whisper STT, `BeatNet`)
//! run in a Python process spawned by the engine (Research 04 §3). The
//! engine writes one [`SidecarRequest`] as a single JSON line to the
//! child's stdin, closes stdin, and reads one [`SidecarResponse`] JSON line
//! back from stdout. The child exits after responding.
//!
//! ## No silent fallback (Linus rule)
//!
//! Every failure mode surfaces as a distinct [`AiError`] variant — spawn
//! failure ([`AiError::SidecarLaunchFailed`]), non-zero exit / closed pipe
//! before a complete response ([`AiError::ProcessExited`]), or malformed
//! JSON ([`AiError::SidecarProtocol`]). There is no broad catch that
//! swallows a crash into an empty result.

use std::io::Write;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AiError;

/// Deterministic request envelope written to the sidecar's stdin.
///
/// Field order is pinned by the struct definition; `serde_json` emits keys
/// in declaration order, so the wire bytes are stable for a given payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidecarRequest {
    /// Protocol op (e.g. `"stt"`, `"beatnet"`). Routes the sidecar handler.
    pub op: String,
    /// Op-specific parameters. Opaque to the transport layer.
    pub params: Value,
}

impl SidecarRequest {
    /// Build a request for `op` with `params`.
    #[must_use]
    pub fn new(op: impl Into<String>, params: Value) -> Self {
        Self {
            op: op.into(),
            params,
        }
    }
}

/// Deterministic response envelope read from the sidecar's stdout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidecarResponse {
    /// Echoed op, for request/response correlation.
    pub op: String,
    /// Op-specific result payload.
    pub result: Value,
}

/// Spawn `program args…`, write `request` as one JSON line to its stdin,
/// and read one [`SidecarResponse`] JSON line back from its stdout.
///
/// The child is expected to consume exactly one request and exit. This is a
/// blocking call intended for the engine's worker-thread context.
///
/// # Errors
///
/// - [`AiError::SidecarLaunchFailed`] — the process could not be spawned
///   (binary missing, exec permission denied) or its stdin/stdout pipes
///   could not be opened.
/// - [`AiError::ProcessExited`] — the child exited with a non-zero status,
///   producing the captured stderr tail.
/// - [`AiError::SidecarProtocol`] — the request could not be serialized, or
///   the child's stdout was not a single valid [`SidecarResponse`] JSON
///   document.
pub fn run_sidecar(
    program: &str,
    args: &[&str],
    request: &SidecarRequest,
) -> Result<SidecarResponse, AiError> {
    let request_line = serde_json::to_string(request).map_err(|err| AiError::SidecarProtocol {
        detail: format!("request serialize failed: {err}"),
    })?;

    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| AiError::SidecarLaunchFailed {
            detail: format!("spawn `{program}` failed: {err}"),
        })?;

    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| AiError::SidecarLaunchFailed {
                detail: "child stdin pipe was not captured".to_string(),
            })?;
        stdin
            .write_all(request_line.as_bytes())
            .and_then(|()| stdin.write_all(b"\n"))
            .map_err(|err| AiError::ProcessExited {
                detail: format!("writing request to sidecar stdin failed: {err}"),
            })?;
        // stdin dropped here -> EOF, so the child's read side unblocks.
    }

    let output = child
        .wait_with_output()
        .map_err(|err| AiError::ProcessExited {
            detail: format!("waiting for sidecar exit failed: {err}"),
        })?;

    if !output.status.success() {
        let stderr_tail = String::from_utf8_lossy(&output.stderr);
        return Err(AiError::ProcessExited {
            detail: format!(
                "sidecar `{program}` exited with {}: {}",
                output.status,
                stderr_tail.trim()
            ),
        });
    }

    let stdout = String::from_utf8(output.stdout).map_err(|err| AiError::SidecarProtocol {
        detail: format!("sidecar stdout was not valid UTF-8: {err}"),
    })?;

    serde_json::from_str::<SidecarResponse>(stdout.trim()).map_err(|err| AiError::SidecarProtocol {
        detail: format!(
            "sidecar response JSON parse failed: {err}; raw=`{}`",
            stdout.trim()
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Write a tiny Python echo sidecar to a temp file and return its path.
    /// The script reads one JSON line, echoes `op`, and returns the squared
    /// `n` from params so the round-trip carries data, not just structure.
    fn write_echo_script() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("echo.py");
        let mut f = std::fs::File::create(&path).expect("create script");
        f.write_all(
            br#"import sys, json
req = json.loads(sys.stdin.readline())
n = req["params"]["n"]
print(json.dumps({"op": req["op"], "result": {"n_squared": n * n}}))
"#,
        )
        .expect("write script");
        (dir, path)
    }

    #[test]
    fn json_round_trip_with_real_python() {
        let (_dir, script) = write_echo_script();
        let req = SidecarRequest::new("stt", json!({ "n": 7 }));
        let resp = run_sidecar("/usr/bin/python3", &[script.to_str().unwrap()], &req)
            .expect("sidecar round-trip");
        assert_eq!(resp.op, "stt");
        assert_eq!(resp.result, json!({ "n_squared": 49 }));
    }

    #[test]
    fn round_trip_is_deterministic() {
        let (_dir, script) = write_echo_script();
        let req = SidecarRequest::new("beatnet", json!({ "n": 3 }));
        let first = run_sidecar("/usr/bin/python3", &[script.to_str().unwrap()], &req).unwrap();
        let second = run_sidecar("/usr/bin/python3", &[script.to_str().unwrap()], &req).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn missing_binary_surfaces_launch_failed() {
        let req = SidecarRequest::new("stt", json!({}));
        let err = run_sidecar("/nonexistent/python-binary", &[], &req).unwrap_err();
        assert!(
            matches!(err, AiError::SidecarLaunchFailed { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn nonzero_exit_surfaces_process_exited() {
        // `python3 -c "import sys; sys.exit(3)"` reads nothing and exits 3.
        let req = SidecarRequest::new("stt", json!({}));
        let err =
            run_sidecar("/usr/bin/python3", &["-c", "import sys; sys.exit(3)"], &req).unwrap_err();
        assert!(matches!(err, AiError::ProcessExited { .. }), "got {err:?}");
    }

    #[test]
    fn malformed_response_surfaces_protocol_error() {
        // Emits non-JSON on stdout and exits 0.
        let req = SidecarRequest::new("stt", json!({}));
        let err = run_sidecar(
            "/usr/bin/python3",
            &["-c", "print('not json at all')"],
            &req,
        )
        .unwrap_err();
        assert!(
            matches!(err, AiError::SidecarProtocol { .. }),
            "got {err:?}"
        );
    }
}
