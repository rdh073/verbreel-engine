//! Round-trip + schema-validation + regex + easing-shape tests for
//! the typed [`Keyframe`].

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use verbreel_state::{Easing, Keyframe, KeyframeProperty, Project};
use verbreel_types::{KeyframeId, Tick, UuidV7};

const FIXTURE: &str = include_str!("fixtures/project_with_keyframes.json");

fn known_v7(suffix: &str) -> UuidV7 {
    format!("01890000-0000-7000-8000-{suffix}")
        .parse::<UuidV7>()
        .unwrap_or_else(|e| panic!("invalid UUID suffix {suffix}: {e}"))
}

/// Schema discovery — same logic as the other schema-validation tests.
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
fn round_trip_project_with_keyframes() {
    let p1: Project = serde_json::from_str(FIXTURE).expect("fixture → Project");
    let s = serde_json::to_string_pretty(&p1).expect("Project → JSON");

    let v_in: Value = serde_json::from_str(FIXTURE).expect("fixture → Value");
    let v_out: Value = serde_json::from_str(&s).expect("round-trip JSON → Value");
    assert_eq!(
        v_in, v_out,
        "round-trip Value must equal the with-keyframes fixture"
    );

    let p2: Project = serde_json::from_str(&s).expect("round-trip JSON → Project");
    assert_eq!(p1, p2, "Project PartialEq must hold across the round trip");

    // Sanity: the video clip has 4 typed keyframes.
    let video_clip = &p1.tracks[0].clips[0];
    assert_eq!(video_clip.keyframes.len(), 4, "video clip has 4 keyframes");
    assert_eq!(video_clip.keyframes[0].property.as_str(), "transform.x");
    assert!(matches!(video_clip.keyframes[0].easing, Easing::Linear));
    assert!(matches!(
        video_clip.keyframes[1].easing,
        Easing::CubicBezier { .. }
    ));
    assert!(matches!(video_clip.keyframes[2].easing, Easing::EaseIn));
    assert!(matches!(video_clip.keyframes[3].easing, Easing::EaseInOut));
    assert!(
        video_clip.keyframes[3]
            .property
            .as_str()
            .starts_with("effects["),
        "fourth keyframe targets an effect param path"
    );
}

#[test]
fn schema_validates_project_with_keyframes() {
    let Some(schema_path) = find_schema() else {
        println!(
            "skipping schema_validates_project_with_keyframes: VERBREEL_SPEC_DIR unset and no \
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
        panic!("with-keyframes fixture must validate against project-schema.json: {errors}");
    }

    let project: Project = serde_json::from_str(FIXTURE).expect("fixture → Project");
    let typed_value = serde_json::to_value(&project).expect("Project → Value");
    if let Err(errors) = validator.validate(&typed_value) {
        panic!("typed Project re-serialized must validate against project-schema.json: {errors}");
    }
}

#[test]
fn keyframe_minimal_round_trip() {
    let id = KeyframeId::from_uuid_v7(known_v7("0000000000a0"));
    let property = KeyframeProperty::new("opacity".to_string()).expect("valid property");
    let kf = Keyframe::new(id, property, Tick::new(0), json!(1.0));

    // Defaults from Keyframe::new.
    assert!(matches!(kf.easing, Easing::Linear));

    let s = serde_json::to_string(&kf).expect("Keyframe → JSON");
    let back: Keyframe = serde_json::from_str(&s).expect("JSON → Keyframe");
    assert_eq!(kf, back);

    let v: Value = serde_json::from_str(&s).expect("JSON → Value");
    assert_eq!(v["property"], "opacity");
    assert_eq!(v["time_tk"], 0);
    assert_eq!(v["value"], 1.0);
    assert_eq!(v["easing"], "linear");
    assert!(
        v.get("bezier").is_none(),
        "minimal Keyframe must not emit bezier"
    );
}

#[test]
fn keyframe_with_easing_round_trip() {
    // 5 non-bezier easings round-trip identically and emit their
    // kebab-case literal at the on-disk level.
    let cases = [
        (Easing::Linear, "linear"),
        (Easing::EaseIn, "ease-in"),
        (Easing::EaseOut, "ease-out"),
        (Easing::EaseInOut, "ease-in-out"),
        (Easing::Step, "step"),
    ];

    for (easing, literal) in cases {
        let id = KeyframeId::from_uuid_v7(known_v7("0000000000b0"));
        let property = KeyframeProperty::new("transform.x".to_string()).expect("valid property");
        let kf = Keyframe {
            id,
            property,
            time_tk: Tick::new(1000),
            value: json!(42.0),
            easing,
        };

        let s = serde_json::to_string(&kf).expect("Keyframe → JSON");
        let back: Keyframe = serde_json::from_str(&s).expect("JSON → Keyframe");
        assert_eq!(kf, back, "round-trip {literal} must equal itself");

        let v: Value = serde_json::from_str(&s).expect("JSON → Value");
        assert_eq!(
            v["easing"], literal,
            "easing literal must serialize as {literal}"
        );
        assert!(
            v.get("bezier").is_none(),
            "{literal} keyframe must not emit bezier (no leftover)"
        );
    }
}

#[test]
fn keyframe_with_cubic_bezier_round_trip() {
    let id = KeyframeId::from_uuid_v7(known_v7("0000000000c0"));
    let property = KeyframeProperty::new("opacity".to_string()).expect("valid property");
    let kf = Keyframe {
        id,
        property,
        time_tk: Tick::new(240_000),
        value: json!(0.5),
        easing: Easing::CubicBezier {
            bezier: [0.25, 0.1, 0.25, 1.0],
        },
    };

    let s = serde_json::to_string(&kf).expect("Keyframe → JSON");
    let back: Keyframe = serde_json::from_str(&s).expect("JSON → Keyframe");
    assert_eq!(kf, back, "cubic-bezier Keyframe must round-trip");

    // Pull the bezier control points back out of the typed variant.
    if let Easing::CubicBezier { bezier } = back.easing {
        assert!((bezier[0] - 0.25).abs() < f64::EPSILON);
        assert!((bezier[1] - 0.1).abs() < f64::EPSILON);
        assert!((bezier[2] - 0.25).abs() < f64::EPSILON);
        assert!((bezier[3] - 1.0).abs() < f64::EPSILON);
    } else {
        panic!("expected CubicBezier variant, got {:?}", back.easing);
    }
}

#[test]
fn easing_serde_shape_cubic_bezier() {
    // The whole point of the KeyframeSerde adapter: on-disk shape is
    // FLAT — `"easing": "cubic-bezier"` + `"bezier": [...]` at the
    // Keyframe level, NEVER nested under an "easing" object.
    let id = KeyframeId::from_uuid_v7(known_v7("0000000000d0"));
    let property = KeyframeProperty::new("transform.x".to_string()).expect("valid property");
    let kf = Keyframe {
        id,
        property,
        time_tk: Tick::new(100),
        value: json!(50.0),
        easing: Easing::CubicBezier {
            bezier: [0.0, 0.0, 1.0, 1.0],
        },
    };

    let v = serde_json::to_value(&kf).expect("Keyframe → Value");
    assert_eq!(v["easing"], "cubic-bezier", "easing is a flat string");
    assert!(
        v["bezier"].is_array(),
        "bezier is a flat array at Keyframe level"
    );
    assert_eq!(v["bezier"][0], 0.0);
    assert_eq!(v["bezier"][1], 0.0);
    assert_eq!(v["bezier"][2], 1.0);
    assert_eq!(v["bezier"][3], 1.0);

    // Ensure NO nested "easing" object exists with sub-fields.
    let easing_field = &v["easing"];
    assert!(
        easing_field.is_string(),
        "easing must be a JSON string, not a nested object"
    );
}

#[test]
fn keyframe_property_accepts_all_schema_forms() {
    let accepted = [
        // transform.<leaf> — all 9
        "transform.x",
        "transform.y",
        "transform.scale_x",
        "transform.scale_y",
        "transform.rotation_deg",
        "transform.anchor_x",
        "transform.anchor_y",
        "transform.skew_x_deg",
        "transform.skew_y_deg",
        // bare leaves
        "opacity",
        "volume",
        // mask leaves
        "mask.feather_px",
        "mask.params.x",
        "mask.params.y",
        "mask.params.w",
        "mask.params.h",
        "mask.params.cx",
        "mask.params.cy",
        "mask.params.rx",
        "mask.params.ry",
        "mask.params.threshold",
        // effects[<uuidv7>].params.<dotted>
        "effects[01890000-0000-7000-8000-000000000010].params.foo",
        "effects[01890000-0000-7000-8000-000000000010].params.foo.bar",
        "effects[01890000-0000-7000-8000-000000000010].params.radius_px",
    ];
    for s in accepted {
        KeyframeProperty::new(s.to_string())
            .unwrap_or_else(|e| panic!("regex must accept {s:?}: {e}"));
    }
}

#[test]
fn keyframe_property_rejects_invalid() {
    let rejected = [
        ("", "empty string"),
        ("unknown_field", "no pattern branch matches"),
        ("transform.speed", "speed not in transform leaf set"),
        (
            "effects[NOT-A-UUID].params.x",
            "effects[...] uuid must match the v7 regex",
        ),
        (
            // v4 UUID: third group starts with 4, regex requires 7
            "effects[550e8400-e29b-41d4-a716-446655440000].params.x",
            "v4 UUID rejected by version nibble in regex",
        ),
        (
            "mask.params.unknown",
            "mask.params leaf not in {x,y,w,h,cx,cy,rx,ry,threshold}",
        ),
        (
            "effects[01890000-0000-7000-8000-000000000010].params",
            "effects[<uuid>].params alone — needs at least one dotted segment",
        ),
        (
            "effects[01890000-0000-7000-8000-000000000010].params.",
            "trailing dot without identifier",
        ),
    ];
    for (s, why) in rejected {
        let res = KeyframeProperty::new(s.to_string());
        assert!(
            res.is_err(),
            "regex must reject {s:?} (reason: {why}) but accepted it"
        );
    }
}

#[test]
fn keyframe_id_round_trip() {
    let raw = "01890000-0000-7000-8000-0000000000aa";
    let id: KeyframeId = raw.parse().expect("parse known v7");
    let s = serde_json::to_string(&id).expect("KeyframeId → JSON");
    assert_eq!(s, format!("\"{raw}\""));
    let back: KeyframeId = serde_json::from_str(&s).expect("JSON → KeyframeId");
    assert_eq!(id, back);

    // v4 must be rejected at the UUIDv7 layer.
    let v4 = "550e8400-e29b-41d4-a716-446655440000";
    serde_json::from_value::<KeyframeId>(json!(v4)).expect_err("v4 must be rejected at KeyframeId");
}

#[test]
fn easing_default_is_linear() {
    // Schema default for easing is "linear". If a deserialized
    // Keyframe omits easing, the adapter falls back to "linear" and
    // produces Easing::Linear.
    let v = json!({
        "id": "01890000-0000-7000-8000-0000000000e0",
        "property": "transform.x",
        "time_tk": 0,
        "value": 0.0
    });
    let kf: Keyframe =
        serde_json::from_value(v).expect("Keyframe with no easing field must default to linear");
    assert!(
        matches!(kf.easing, Easing::Linear),
        "easing default must be Linear, got {:?}",
        kf.easing
    );

    // Default::default() on Easing is also Linear.
    assert!(matches!(Easing::default(), Easing::Linear));
}

#[test]
fn keyframe_cubic_bezier_missing_bezier_rejected() {
    // Schema if/then: easing="cubic-bezier" REQUIRES bezier.
    // Adapter must reject the half-state.
    let v = json!({
        "id": "01890000-0000-7000-8000-0000000000f0",
        "property": "opacity",
        "time_tk": 0,
        "value": 0.0,
        "easing": "cubic-bezier"
    });
    let err =
        serde_json::from_value::<Keyframe>(v).expect_err("cubic-bezier without bezier must fail");
    assert!(
        format!("{err}").contains("cubic-bezier requires bezier"),
        "error message must reference the schema if/then violation, got: {err}"
    );
}

#[test]
fn keyframe_unknown_easing_rejected() {
    // Non-enum easing literal is rejected by the adapter.
    let v = json!({
        "id": "01890000-0000-7000-8000-0000000000f1",
        "property": "opacity",
        "time_tk": 0,
        "value": 0.0,
        "easing": "bounce-out"
    });
    let err = serde_json::from_value::<Keyframe>(v).expect_err("unknown easing must fail");
    assert!(
        format!("{err}").contains("bounce-out"),
        "error message must mention the offending easing literal, got: {err}"
    );
}
