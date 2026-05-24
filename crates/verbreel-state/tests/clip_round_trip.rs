//! Round-trip + schema validation tests for the typed [`Clip`].

use std::path::{Path, PathBuf};

use serde_json::Value;
use verbreel_state::{
    AssetRef, BlendMode, Clip, ClipMask, FadeCurve, MaskKind, Project, SpeedCurvePoint,
    TextElement, Transform,
};
use verbreel_types::{AssetId, ClipId, LinkGroupId, Tick, UuidV7};

const FIXTURE: &str = include_str!("fixtures/project_with_clips.json");

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
fn round_trip_project_with_clips() {
    let p1: Project = serde_json::from_str(FIXTURE).expect("fixture → Project");
    let s = serde_json::to_string_pretty(&p1).expect("Project → JSON");

    let v_in: Value = serde_json::from_str(FIXTURE).expect("fixture → Value");
    let v_out: Value = serde_json::from_str(&s).expect("round-trip JSON → Value");
    assert_eq!(
        v_in, v_out,
        "round-trip Value must equal the populated-clip fixture"
    );

    let p2: Project = serde_json::from_str(&s).expect("round-trip JSON → Project");
    assert_eq!(p1, p2, "Project PartialEq must hold across the round trip");

    // Sanity: 3 tracks, 1 video clip, 0 audio clips, 1 text clip.
    assert_eq!(p1.tracks.len(), 3);
    assert_eq!(p1.tracks[0].clips.len(), 1, "video track has 1 clip");
    assert_eq!(p1.tracks[1].clips.len(), 0, "audio track has 0 clips");
    assert_eq!(p1.tracks[2].clips.len(), 1, "text track has 1 clip");
}

#[test]
fn schema_validates_project_with_clips() {
    let Some(schema_path) = find_schema() else {
        println!(
            "skipping schema_validates_project_with_clips: VERBREEL_SPEC_DIR unset and no \
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
        panic!("populated-clip fixture must validate against project-schema.json: {errors}");
    }

    let project: Project = serde_json::from_str(FIXTURE).expect("fixture → Project");
    let typed_value = serde_json::to_value(&project).expect("Project → Value");
    if let Err(errors) = validator.validate(&typed_value) {
        panic!("typed Project re-serialized must validate against project-schema.json: {errors}");
    }
}

fn known_v7(suffix: &str) -> UuidV7 {
    format!("0190b8d3-15e3-7000-bd00-{suffix}")
        .parse::<UuidV7>()
        .unwrap_or_else(|e| panic!("invalid UUID suffix {suffix}: {e}"))
}

#[test]
fn clip_required_fields_round_trip() {
    // Minimal Clip: only the 6 required fields explicit; everything
    // else comes from `Default` via serde defaults on deserialize.
    let json = serde_json::json!({
        "id": "0190b8d3-15e3-7000-bd00-000000000c01",
        "name": "minimal",
        "asset_id": "0190b8d3-15e3-7000-bd00-0000000000a1",
        "track_position_tk": 0,
        "source_in_tk": 0,
        "source_out_tk": 1000
    });
    let c: Clip = serde_json::from_value(json).expect("minimal Clip must deserialize");
    // Defaults match the schema defaults.
    assert!((c.speed - 1.0).abs() < f64::EPSILON);
    assert!((c.opacity - 1.0).abs() < f64::EPSILON);
    assert!((c.volume - 1.0).abs() < f64::EPSILON);
    assert!(!c.reversed);
    assert!(!c.locked);
    assert_eq!(c.fade_in_tk, Tick::ZERO);
    assert_eq!(c.fade_out_tk, Tick::ZERO);
    assert_eq!(c.fade_in_curve, FadeCurve::Linear);
    assert_eq!(c.fade_out_curve, FadeCurve::Linear);
    assert_eq!(c.blend_mode, BlendMode::Normal);
    assert!(c.text.is_none());
    assert!(c.mask.is_none());
    assert!(c.speed_curve.is_none());
    assert!(c.link_group.is_none());
    assert_eq!(c.transform, Transform::default());

    // Round-trip back to JSON. Since `transform` always serializes
    // (no `skip_serializing_if`), the output includes its full
    // defaulted shape — that's the §0.5.2 canonical form.
    let back = serde_json::to_value(&c).expect("Clip → Value");
    assert_eq!(back["id"], "0190b8d3-15e3-7000-bd00-000000000c01");
    assert_eq!(back["transform"]["anchor_x"], 0.5);
    assert_eq!(back["transform"]["scale_x"], 1.0);
    assert_eq!(back["blend_mode"], "normal");
}

#[test]
fn clip_full_fields_round_trip() {
    // Construct a Clip with every optional field populated.
    let id = ClipId::from_uuid_v7(known_v7("000000000c01"));
    let asset = AssetId::from_uuid_v7(known_v7("0000000000a1"));
    let link = LinkGroupId::from_uuid_v7(known_v7("00000000beef"));

    let clip = Clip {
        id,
        name: "full".to_string(),
        asset_id: AssetRef::from_id(asset),
        track_position_tk: Tick::new(10),
        source_in_tk: Tick::new(0),
        source_out_tk: Tick::new(2400000),
        speed: 2.0,
        reversed: true,
        transform: Transform {
            x: 100.0,
            y: 200.0,
            scale_x: 1.5,
            scale_y: 1.5,
            rotation_deg: 45.0,
            anchor_x: 0.0,
            anchor_y: 0.0,
            skew_x_deg: 10.0,
            skew_y_deg: 5.0,
            flip_h: true,
            flip_v: false,
        },
        opacity: 0.5,
        volume: 0.75,
        fade_in_tk: Tick::new(24000),
        fade_out_tk: Tick::new(48000),
        fade_in_curve: FadeCurve::Exp,
        fade_out_curve: FadeCurve::Log,
        effects: vec![],
        keyframes: vec![],
        text: None,
        locked: true,
        link_group: Some(link),
        blend_mode: BlendMode::SoftLight,
        mask: Some(ClipMask {
            kind: MaskKind::Rect,
            params: serde_json::Map::new(),
            feather_px: 2.5,
            inverted: true,
        }),
        speed_curve: Some(vec![
            SpeedCurvePoint {
                time_tk: Tick::new(0),
                factor: 1.0,
            },
            SpeedCurvePoint {
                time_tk: Tick::new(240000),
                factor: 2.0,
            },
        ]),
    };

    let s = serde_json::to_string(&clip).expect("Clip → JSON");
    let back: Clip = serde_json::from_str(&s).expect("JSON → Clip");
    assert_eq!(clip, back, "full-fields Clip must round-trip identically");

    // Spot-check the on-wire shape for kebab-case + lowercase enums.
    let v: Value = serde_json::from_str(&s).expect("JSON → Value");
    assert_eq!(v["blend_mode"], "soft-light");
    assert_eq!(v["fade_in_curve"], "exp");
    assert_eq!(v["fade_out_curve"], "log");
    assert_eq!(v["mask"]["kind"], "rect");
    assert_eq!(v["speed_curve"][0]["factor"], 1.0);
    assert_eq!(v["speed_curve"][1]["factor"], 2.0);
}

#[test]
fn text_clip_round_trip() {
    // Text clip uses the nil-UUID AssetRef + a populated TextElement.
    let id = ClipId::from_uuid_v7(known_v7("000000000c02"));
    let clip = Clip {
        id,
        name: "text".to_string(),
        asset_id: AssetRef::nil(),
        track_position_tk: Tick::ZERO,
        source_in_tk: Tick::ZERO,
        source_out_tk: Tick::new(480000),
        speed: 1.0,
        reversed: false,
        transform: Transform::default(),
        opacity: 1.0,
        volume: 1.0,
        fade_in_tk: Tick::ZERO,
        fade_out_tk: Tick::ZERO,
        fade_in_curve: FadeCurve::Linear,
        fade_out_curve: FadeCurve::Linear,
        effects: vec![],
        keyframes: vec![],
        text: Some(TextElement {
            content: "Hello".to_string(),
            ..Default::default()
        }),
        locked: false,
        link_group: None,
        blend_mode: BlendMode::Normal,
        mask: None,
        speed_curve: None,
    };

    let s = serde_json::to_string(&clip).expect("text Clip → JSON");
    let back: Clip = serde_json::from_str(&s).expect("JSON → Clip");
    assert_eq!(clip, back, "text Clip must round-trip identically");

    let v: Value = serde_json::from_str(&s).expect("JSON → Value");
    assert_eq!(
        v["asset_id"], "00000000-0000-0000-0000-000000000000",
        "text clip asset_id must serialize as the nil UUID"
    );
    assert_eq!(v["text"]["content"], "Hello");
    assert_eq!(
        v["text"]["color"], "#ffffffff",
        "TextElement default color is #ffffffff per schema"
    );
}
