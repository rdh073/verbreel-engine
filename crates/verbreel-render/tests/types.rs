//! Type-surface tests for verbreel-render v0.

use verbreel_render::{Pipeline, RenderError, RenderPreset};

// --- RenderPreset --------------------------------------------------------

#[test]
fn render_preset_has_two_variants() {
    // Compile-time exhaustiveness check via match.
    fn classify(p: RenderPreset) -> &'static str {
        match p {
            RenderPreset::Deterministic => "deterministic",
            RenderPreset::Performance => "performance",
        }
    }
    assert_eq!(classify(RenderPreset::Deterministic), "deterministic");
    assert_eq!(classify(RenderPreset::Performance), "performance");
}

#[test]
fn render_preset_as_str_matches_spec_ids() {
    assert_eq!(RenderPreset::Deterministic.as_str(), "deterministic");
    assert_eq!(RenderPreset::Performance.as_str(), "performance");
}

#[test]
fn render_preset_is_copy_and_clone() {
    let a = RenderPreset::Deterministic;
    let b = a;
    let c = a;
    assert_eq!(a, b);
    assert_eq!(a, c);
}

#[test]
fn render_preset_is_hashable() {
    use std::collections::HashSet;
    let mut s: HashSet<RenderPreset> = HashSet::new();
    s.insert(RenderPreset::Deterministic);
    s.insert(RenderPreset::Performance);
    assert_eq!(s.len(), 2);
    s.insert(RenderPreset::Deterministic);
    assert_eq!(s.len(), 2, "duplicate insert must not grow the set");
}

#[test]
fn render_preset_distinct_variants_compare_unequal() {
    assert_ne!(RenderPreset::Deterministic, RenderPreset::Performance);
}

// --- Pipeline ------------------------------------------------------------

#[test]
fn pipeline_new_returns_handle_bound_to_preset() {
    let p = Pipeline::new(RenderPreset::Deterministic);
    assert_eq!(p.preset(), RenderPreset::Deterministic);
}

#[test]
fn pipeline_new_with_performance_preset() {
    let p = Pipeline::new(RenderPreset::Performance);
    assert_eq!(p.preset(), RenderPreset::Performance);
}

#[test]
fn pipeline_preset_accessor_is_pure_read() {
    // Two reads of the same field must return the same value.
    let p = Pipeline::new(RenderPreset::Deterministic);
    assert_eq!(p.preset(), p.preset());
}

#[test]
fn pipeline_is_send_and_sync() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    assert_send::<Pipeline>();
    assert_sync::<Pipeline>();
}

#[test]
fn pipeline_is_debug() {
    let p = Pipeline::new(RenderPreset::Deterministic);
    let rendered = format!("{p:?}");
    assert!(
        rendered.contains("Pipeline"),
        "Debug must name the type, got: {rendered}"
    );
}

// --- RenderError ---------------------------------------------------------

#[test]
fn render_error_not_yet_implemented_variant() {
    let err = RenderError::NotYetImplemented {
        detail: "x".to_string(),
    };
    assert!(matches!(err, RenderError::NotYetImplemented { .. }));
}

#[test]
fn render_error_surface_lost_variant() {
    let err = RenderError::SurfaceLost {
        detail: "GPU reset".to_string(),
    };
    assert!(matches!(err, RenderError::SurfaceLost { .. }));
}

#[test]
fn render_error_shader_compile_variant() {
    let err = RenderError::ShaderCompile {
        detail: "naga: undefined identifier".to_string(),
    };
    assert!(matches!(err, RenderError::ShaderCompile { .. }));
}

#[test]
fn render_error_display_includes_detail() {
    let err = RenderError::NotYetImplemented {
        detail: "hello detail".to_string(),
    };
    assert!(err.to_string().contains("hello detail"));
}

#[test]
fn render_error_distinct_variants_compare_unequal() {
    let a = RenderError::NotYetImplemented {
        detail: "x".to_string(),
    };
    let b = RenderError::SurfaceLost {
        detail: "x".to_string(),
    };
    let c = RenderError::ShaderCompile {
        detail: "x".to_string(),
    };
    assert_ne!(a, b);
    assert_ne!(a, c);
    assert_ne!(b, c);
}
