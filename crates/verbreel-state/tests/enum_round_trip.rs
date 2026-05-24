//! `BlendMode` / `FadeCurve` / `MaskKind` enum round-trips. Locks
//! that serde's `rename_all` rules produce the schema-exact strings
//! ("soft-light" not "soft_light" etc.).

use serde_json::{Value, json};
use verbreel_state::{BlendMode, FadeCurve, MaskKind};

fn round_trip<T>(v: T, expected_string: &str)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let s = serde_json::to_value(&v).expect("variant → Value");
    assert_eq!(
        s,
        Value::String(expected_string.to_string()),
        "variant must serialize as the schema-exact literal"
    );
    let back: T = serde_json::from_value(json!(expected_string))
        .unwrap_or_else(|e| panic!("schema string {expected_string:?} must deserialize: {e}"));
    assert_eq!(back, v);
}

#[test]
fn fade_curve_round_trip_all_3() {
    round_trip(FadeCurve::Linear, "linear");
    round_trip(FadeCurve::Exp, "exp");
    round_trip(FadeCurve::Log, "log");
}

#[test]
fn blend_mode_round_trip_all_11() {
    round_trip(BlendMode::Normal, "normal");
    round_trip(BlendMode::Multiply, "multiply");
    round_trip(BlendMode::Screen, "screen");
    round_trip(BlendMode::Overlay, "overlay");
    round_trip(BlendMode::SoftLight, "soft-light");
    round_trip(BlendMode::HardLight, "hard-light");
    round_trip(BlendMode::Darken, "darken");
    round_trip(BlendMode::Lighten, "lighten");
    round_trip(BlendMode::Difference, "difference");
    round_trip(BlendMode::ColorDodge, "color-dodge");
    round_trip(BlendMode::ColorBurn, "color-burn");
}

#[test]
fn mask_kind_round_trip_all_4() {
    round_trip(MaskKind::Rect, "rect");
    round_trip(MaskKind::Ellipse, "ellipse");
    round_trip(MaskKind::Polygon, "polygon");
    round_trip(MaskKind::Asset, "asset");
}
