//! Hardware acceleration policy contract tests.

use verbreel_render::{
    HwAccelKind, RenderPreset, V1_HWACCEL_PRIORITY, hwaccel_priority_for_preset,
};

#[test]
fn v1_priority_order_is_pinned_to_option_a() {
    assert_eq!(
        V1_HWACCEL_PRIORITY,
        [
            HwAccelKind::Nvenc,
            HwAccelKind::Vaapi,
            HwAccelKind::VideoToolbox
        ]
    );
}

#[test]
fn v1_priority_wire_names_are_stable() {
    let names: Vec<&str> = V1_HWACCEL_PRIORITY
        .iter()
        .map(HwAccelKind::as_str)
        .collect();
    assert_eq!(names, vec!["NVENC", "VAAPI", "VideoToolbox"]);
}

#[test]
fn deterministic_preset_forbids_hwaccel() {
    let allowed = hwaccel_priority_for_preset(RenderPreset::Deterministic);
    assert!(
        allowed.is_empty(),
        "deterministic mode must be software-only, got: {allowed:?}"
    );
}

#[test]
fn performance_preset_uses_v1_priority() {
    let allowed = hwaccel_priority_for_preset(RenderPreset::Performance);
    assert_eq!(allowed, &V1_HWACCEL_PRIORITY);
}
