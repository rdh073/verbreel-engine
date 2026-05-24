//! Subtype `Default` impls must match the schema defaults bit-for-bit.

use serde_json::{Value, json};
use verbreel_state::{Shadow, TextElement, Transform};

#[test]
fn transform_default_matches_schema() {
    let v = serde_json::to_value(Transform::default()).expect("Transform → Value");
    let expected = json!({
        "x": 0.0, "y": 0.0,
        "scale_x": 1.0, "scale_y": 1.0,
        "rotation_deg": 0.0,
        "anchor_x": 0.5, "anchor_y": 0.5,
        "skew_x_deg": 0.0, "skew_y_deg": 0.0,
        "flip_h": false, "flip_v": false
    });
    assert_eq!(
        v, expected,
        "Transform::default() must match the schema-listed defaults"
    );
}

#[test]
fn shadow_default_matches_schema() {
    let v = serde_json::to_value(Shadow::default()).expect("Shadow → Value");
    let expected = json!({
        "color": "#000000aa",
        "blur_px": 4.0,
        "offset_x": 0.0,
        "offset_y": 2.0
    });
    assert_eq!(
        v, expected,
        "Shadow::default() must match the schema-listed defaults"
    );
}

#[test]
fn text_element_default_matches_schema() {
    // TextElement::default() carries content = "" so a caller has to
    // set it explicitly before serializing. Set "x" and verify the
    // remaining 13 fields all match schema defaults.
    let t = TextElement {
        content: "x".to_string(),
        ..Default::default()
    };
    let v = serde_json::to_value(&t).expect("TextElement → Value");
    let expected = json!({
        "content": "x",
        "font_family": "Inter",
        "font_size_px": 64.0,
        "font_weight": 700,
        "italic": false,
        "color": "#ffffffff",
        "stroke_px": 0.0,
        "align": "center",
        "letter_spacing": 0.0,
        "line_height": 1.2,
        "padding_px": 0.0
    });
    // Optional fields with no schema default (bg_color, stroke_color,
    // shadow) are absent because of skip_serializing_if = Option::is_none.
    let obj = v.as_object().unwrap();
    assert!(
        !obj.contains_key("bg_color"),
        "bg_color (no schema default) must be absent when None"
    );
    assert!(
        !obj.contains_key("stroke_color"),
        "stroke_color (no schema default) must be absent when None"
    );
    assert!(
        !obj.contains_key("shadow"),
        "shadow (no schema default) must be absent when None"
    );
    // Compare remaining keys.
    let obj_no_optional: Value = serde_json::to_value(obj).unwrap();
    assert_eq!(
        obj_no_optional, expected,
        "TextElement defaults must match schema"
    );
}
