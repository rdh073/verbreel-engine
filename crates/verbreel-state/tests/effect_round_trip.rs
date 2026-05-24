//! Round-trip + schema-validation + flatten-shape tests for the typed
//! [`Effect`].

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use verbreel_state::{Effect, EffectKind, EffectWindow, Project};
use verbreel_types::{EffectId, Tick, UuidV7};

const FIXTURE: &str = include_str!("fixtures/project_with_effects.json");

fn known_v7(suffix: &str) -> UuidV7 {
    format!("01890000-0000-7000-8000-{suffix}")
        .parse::<UuidV7>()
        .unwrap_or_else(|e| panic!("invalid UUID suffix {suffix}: {e}"))
}

/// Schema discovery — same logic as `tests/schema_validation.rs`.
fn find_schema() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("VERBREEL_SPEC_DIR") {
        let p = PathBuf::from(dir).join("spec").join("project-schema.json");
        if p.is_file() {
            return Some(p);
        }
    }
    let start = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut cur: &Path = &start;
    loop {
        let candidate = cur
            .join("..")
            .join("verbreel-spec")
            .join("spec")
            .join("project-schema.json");
        if let Ok(canonical) = candidate.canonicalize()
            && canonical.is_file()
        {
            return Some(canonical);
        }
        match cur.parent() {
            Some(parent) => cur = parent,
            None => return None,
        }
    }
}

#[test]
fn round_trip_project_with_effects() {
    let p1: Project = serde_json::from_str(FIXTURE).expect("fixture → Project");
    let s = serde_json::to_string_pretty(&p1).expect("Project → JSON");

    let v_in: Value = serde_json::from_str(FIXTURE).expect("fixture → Value");
    let v_out: Value = serde_json::from_str(&s).expect("round-trip JSON → Value");
    assert_eq!(
        v_in, v_out,
        "round-trip Value must equal the with-effects fixture"
    );

    let p2: Project = serde_json::from_str(&s).expect("round-trip JSON → Project");
    assert_eq!(p1, p2, "Project PartialEq must hold across the round trip");

    // Sanity: the video clip has 2 typed effects.
    let video_clip = &p1.tracks[0].clips[0];
    assert_eq!(video_clip.effects.len(), 2, "video clip has 2 effects");
    assert_eq!(video_clip.effects[0].kind.as_str(), "blur");
    assert_eq!(video_clip.effects[1].kind.as_str(), "lut");
    assert!(video_clip.effects[0].window.is_none(), "blur has no window");
    assert!(
        video_clip.effects[1].window.is_some(),
        "lut has a paired window"
    );
}

#[test]
fn schema_validates_project_with_effects() {
    let Some(schema_path) = find_schema() else {
        println!(
            "skipping schema_validates_project_with_effects: VERBREEL_SPEC_DIR unset and no \
             sibling verbreel-spec checkout"
        );
        return;
    };
    let schema_text = std::fs::read_to_string(&schema_path)
        .unwrap_or_else(|e| panic!("read schema at {}: {e}", schema_path.display()));
    let schema: Value = serde_json::from_str(&schema_text).expect("schema parses");
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");

    let fixture_value: Value = serde_json::from_str(FIXTURE).expect("fixture parses");
    if let Err(errors) = validator.validate(&fixture_value) {
        panic!("with-effects fixture must validate against project-schema.json: {errors}");
    }

    let project: Project = serde_json::from_str(FIXTURE).expect("fixture → Project");
    let typed_value = serde_json::to_value(&project).expect("Project → Value");
    if let Err(errors) = validator.validate(&typed_value) {
        panic!("typed Project re-serialized must validate against project-schema.json: {errors}");
    }
}

#[test]
fn effect_minimal_round_trip() {
    let id = EffectId::from_uuid_v7(known_v7("000000000010"));
    let kind = EffectKind::new("blur".to_string()).expect("non-empty kind");
    let effect = Effect::new(id, kind);

    // Defaults established by `Effect::new`: enabled=true, empty
    // params, no window.
    assert!(effect.enabled);
    assert!(effect.params.is_empty());
    assert!(effect.window.is_none());

    let s = serde_json::to_string(&effect).expect("Effect → JSON");
    let back: Effect = serde_json::from_str(&s).expect("JSON → Effect");
    assert_eq!(effect, back, "minimal Effect must round-trip identically");

    // On-wire shape: id/kind/enabled/params present, no in_tk/out_tk.
    let v: Value = serde_json::from_str(&s).expect("JSON → Value");
    assert!(v.get("id").is_some());
    assert_eq!(v["kind"], "blur");
    assert_eq!(v["enabled"], true);
    assert!(v["params"].is_object());
    assert!(
        v.get("in_tk").is_none(),
        "minimal Effect must not emit in_tk"
    );
    assert!(
        v.get("out_tk").is_none(),
        "minimal Effect must not emit out_tk"
    );
}

#[test]
fn effect_with_window_round_trip() {
    let id = EffectId::from_uuid_v7(known_v7("000000000020"));
    let kind = EffectKind::new("lut".to_string()).expect("non-empty kind");
    let mut effect = Effect::new(id, kind);
    effect.window = Some(EffectWindow {
        in_tk: Tick::new(240_000),
        out_tk: Tick::new(720_000),
    });

    let s = serde_json::to_string(&effect).expect("Effect → JSON");
    let back: Effect = serde_json::from_str(&s).expect("JSON → Effect");
    assert_eq!(effect, back, "windowed Effect must round-trip identically");

    let v: Value = serde_json::from_str(&s).expect("JSON → Value");
    assert_eq!(v["in_tk"], 240_000);
    assert_eq!(v["out_tk"], 720_000);
}

#[test]
fn effect_without_window_round_trip() {
    let id = EffectId::from_uuid_v7(known_v7("000000000030"));
    let kind = EffectKind::new("eq".to_string()).expect("non-empty kind");
    let effect = Effect::new(id, kind);
    assert!(effect.window.is_none(), "Effect::new starts without window");

    let s = serde_json::to_string(&effect).expect("Effect → JSON");
    let v: Value = serde_json::from_str(&s).expect("JSON → Value");
    assert!(
        v.get("in_tk").is_none(),
        "windowless Effect must not emit in_tk"
    );
    assert!(
        v.get("out_tk").is_none(),
        "windowless Effect must not emit out_tk"
    );

    // Deserialize back: window stays absent (None).
    let back: Effect = serde_json::from_str(&s).expect("JSON → Effect");
    assert!(
        back.window.is_none(),
        "round-trip preserves no-window state"
    );
}

#[test]
fn effect_window_flatten_serde_shape() {
    // The whole point of the EffectSerde adapter: on-disk shape is
    // FLAT — `"in_tk"` and `"out_tk"` at the Effect level, NEVER
    // nested under a `"window"` object.
    let id = EffectId::from_uuid_v7(known_v7("000000000040"));
    let kind = EffectKind::new("blur".to_string()).expect("non-empty kind");
    let mut effect = Effect::new(id, kind);
    effect.window = Some(EffectWindow {
        in_tk: Tick::new(100),
        out_tk: Tick::new(200),
    });

    let v = serde_json::to_value(&effect).expect("Effect → Value");
    assert!(
        v.get("in_tk").is_some(),
        "expect flat in_tk at Effect level"
    );
    assert!(
        v.get("out_tk").is_some(),
        "expect flat out_tk at Effect level"
    );
    assert!(
        v.get("window").is_none(),
        "must NOT emit a nested 'window' object — schema is flat"
    );

    // And the value of those fields must equal Tick(100)/Tick(200).
    assert_eq!(v["in_tk"], 100);
    assert_eq!(v["out_tk"], 200);
}

#[test]
fn effect_window_dependent_required_violation_rejected() {
    // Schema's dependentRequired says in_tk and out_tk must both
    // appear or both be absent. The serde adapter must reject the
    // half-window cases.
    let half_in = json!({
        "id": "01890000-0000-7000-8000-000000000050",
        "kind": "blur",
        "params": {},
        "in_tk": 240000
    });
    let err = serde_json::from_value::<Effect>(half_in).expect_err("in_tk alone must be rejected");
    assert!(
        format!("{err}").contains("dependentRequired"),
        "error message must reference the dependentRequired violation, got: {err}"
    );

    let half_out = json!({
        "id": "01890000-0000-7000-8000-000000000051",
        "kind": "blur",
        "params": {},
        "out_tk": 720000
    });
    let err =
        serde_json::from_value::<Effect>(half_out).expect_err("out_tk alone must be rejected");
    assert!(
        format!("{err}").contains("dependentRequired"),
        "error message must reference the dependentRequired violation, got: {err}"
    );
}

#[test]
fn effect_kind_rejects_empty_string() {
    let err = EffectKind::new(String::new()).expect_err("empty EffectKind must be rejected");
    assert!(
        format!("{err}").contains("minLength: 1"),
        "error must mention minLength, got: {err}"
    );

    // Same rejection via serde (try_from): empty string parses as
    // String then EffectKind::try_from rejects.
    let err = serde_json::from_value::<EffectKind>(json!("")).expect_err("serde rejects empty");
    assert!(
        format!("{err}").contains("minLength: 1"),
        "serde error must mention minLength, got: {err}"
    );
}

#[test]
fn effect_id_round_trip() {
    // EffectId is a `UUIDv7` newtype; its serde shape is the bare
    // hyphenated UUID string. Round-trip a known v7.
    let raw = "01890000-0000-7000-8000-0000000000aa";
    let id: EffectId = raw.parse().expect("parse known v7");
    let s = serde_json::to_string(&id).expect("EffectId → JSON");
    assert_eq!(s, format!("\"{raw}\""));
    let back: EffectId = serde_json::from_str(&s).expect("JSON → EffectId");
    assert_eq!(id, back);

    // v4 must be rejected at the UUIDv7 layer.
    let v4 = "550e8400-e29b-41d4-a716-446655440000";
    serde_json::from_value::<EffectId>(json!(v4)).expect_err("v4 must be rejected at EffectId");
}

#[test]
fn effect_enabled_default_is_true() {
    // Schema default is `enabled: true`. If a deserialized Effect
    // omits the field, serde should fill in `true`.
    let v = json!({
        "id": "01890000-0000-7000-8000-000000000060",
        "kind": "blur",
        "params": {}
    });
    let effect: Effect =
        serde_json::from_value(v).expect("Effect with no enabled field must default to true");
    assert!(effect.enabled, "enabled default must be true per schema");

    // Disabled stays disabled.
    let v = json!({
        "id": "01890000-0000-7000-8000-000000000061",
        "kind": "blur",
        "enabled": false,
        "params": {}
    });
    let effect: Effect = serde_json::from_value(v).expect("Effect with enabled=false");
    assert!(!effect.enabled);
}
