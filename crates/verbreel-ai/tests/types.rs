//! Type-surface tests for verbreel-ai v0.

use verbreel_ai::{AiError, Capability, Provider};

// --- Capability ----------------------------------------------------------

#[test]
fn capability_has_four_variants() {
    fn classify(c: Capability) -> &'static str {
        match c {
            Capability::Whisper => "w",
            Capability::Onset => "o",
            Capability::Tempo => "t",
            Capability::BeatNet => "b",
        }
    }
    assert_eq!(classify(Capability::Whisper), "w");
    assert_eq!(classify(Capability::Onset), "o");
    assert_eq!(classify(Capability::Tempo), "t");
    assert_eq!(classify(Capability::BeatNet), "b");
}

#[test]
fn capability_as_str_matches_spec_ids() {
    assert_eq!(Capability::Whisper.as_str(), "whisper");
    assert_eq!(Capability::Onset.as_str(), "onset");
    assert_eq!(Capability::Tempo.as_str(), "tempo");
    assert_eq!(Capability::BeatNet.as_str(), "beatnet");
}

#[test]
fn capability_is_sidecar_only_for_beatnet() {
    assert!(!Capability::Whisper.is_sidecar());
    assert!(!Capability::Onset.is_sidecar());
    assert!(!Capability::Tempo.is_sidecar());
    assert!(Capability::BeatNet.is_sidecar());
}

#[test]
fn capability_is_copy_and_clone() {
    let a = Capability::Whisper;
    let b = a;
    let c = a;
    assert_eq!(a, b);
    assert_eq!(a, c);
}

#[test]
fn capability_is_hashable() {
    use std::collections::HashSet;
    let mut s: HashSet<Capability> = HashSet::new();
    s.insert(Capability::Whisper);
    s.insert(Capability::Onset);
    s.insert(Capability::Tempo);
    s.insert(Capability::BeatNet);
    s.insert(Capability::Whisper);
    assert_eq!(s.len(), 4);
}

// --- Provider ------------------------------------------------------------

#[test]
fn provider_new_binds_capability() {
    let p = Provider::new(Capability::Whisper);
    assert_eq!(p.capability(), Capability::Whisper);
}

#[test]
fn provider_new_with_each_capability() {
    for cap in [
        Capability::Whisper,
        Capability::Onset,
        Capability::Tempo,
        Capability::BeatNet,
    ] {
        let p = Provider::new(cap);
        assert_eq!(p.capability(), cap);
    }
}

#[test]
fn provider_capability_accessor_is_pure_read() {
    let p = Provider::new(Capability::Onset);
    assert_eq!(p.capability(), p.capability());
}

#[test]
fn provider_is_send_and_sync() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    assert_send::<Provider>();
    assert_sync::<Provider>();
}

#[test]
fn provider_is_debug() {
    let p = Provider::new(Capability::Whisper);
    let rendered = format!("{p:?}");
    assert!(
        rendered.contains("Provider"),
        "Debug must name the type, got: {rendered}"
    );
}

// --- AiError -------------------------------------------------------------

#[test]
fn ai_error_not_yet_implemented_variant() {
    let err = AiError::NotYetImplemented {
        detail: "x".to_string(),
    };
    assert!(matches!(err, AiError::NotYetImplemented { .. }));
}

#[test]
fn ai_error_sidecar_launch_failed_variant() {
    let err = AiError::SidecarLaunchFailed {
        detail: "binary missing".to_string(),
    };
    assert!(matches!(err, AiError::SidecarLaunchFailed { .. }));
}

#[test]
fn ai_error_model_load_failed_variant() {
    let err = AiError::ModelLoadFailed {
        detail: "ort: cuda unavailable".to_string(),
    };
    assert!(matches!(err, AiError::ModelLoadFailed { .. }));
}

#[test]
fn ai_error_process_exited_variant() {
    let err = AiError::ProcessExited {
        detail: "exit code 1".to_string(),
    };
    assert!(matches!(err, AiError::ProcessExited { .. }));
}

#[test]
fn ai_error_display_includes_detail() {
    let err = AiError::NotYetImplemented {
        detail: "hello detail".to_string(),
    };
    assert!(err.to_string().contains("hello detail"));
}

#[test]
fn ai_error_distinct_variants_compare_unequal() {
    let a = AiError::NotYetImplemented {
        detail: "x".to_string(),
    };
    let b = AiError::SidecarLaunchFailed {
        detail: "x".to_string(),
    };
    let c = AiError::ModelLoadFailed {
        detail: "x".to_string(),
    };
    let d = AiError::ProcessExited {
        detail: "x".to_string(),
    };
    assert_ne!(a, b);
    assert_ne!(a, c);
    assert_ne!(a, d);
    assert_ne!(b, c);
    assert_ne!(b, d);
    assert_ne!(c, d);
}
