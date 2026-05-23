//! Shared bench helpers for spike S3 — kept inside the lib so unit
//! tests can use them and the example file stays thin.

/// Peak resident set size in kB, parsed from `/proc/self/status:VmPeak`.
/// On Linux this proxy includes any GPU memory mapped into the host
/// address space (CPU-mappable BARs, staging buffers) but does NOT
/// include pure VRAM owned by the GPU. That asymmetry is exactly the
/// signal we want for §11 S3 — gpu-video keeps decoded NV12 frames on
/// the GPU, which never inflates VmPeak; rsmpeg copies them through
/// host memory, which does.
pub fn peak_rss_kb() -> u64 {
    let s = match std::fs::read_to_string("/proc/self/status") {
        Ok(s) => s,
        Err(_) => return 0,
    };
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("VmPeak:")
            && let Some(num) = rest.split_whitespace().next()
            && let Ok(n) = num.parse::<u64>()
        {
            return n;
        }
    }
    0
}
