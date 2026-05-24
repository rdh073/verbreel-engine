//! `Color` newtype validation + lowercase-normalization tests.

use serde_json::json;
use verbreel_state::Color;

#[test]
fn color_accepts_lowercase_input() {
    let c = Color::new("#ff8800ff".to_string()).expect("valid lowercase must parse");
    assert_eq!(c.as_str(), "#ff8800ff");
}

#[test]
fn color_normalizes_to_lowercase() {
    // Schema regex accepts uppercase (`^#[0-9a-fA-F]{8}$`) but spec
    // §0.5.2 mandates lowercase on persist. Construction normalizes.
    let c = Color::new("#FF8800FF".to_string()).expect("uppercase must parse");
    assert_eq!(
        c.as_str(),
        "#ff8800ff",
        "uppercase input must store lowercase"
    );
}

#[test]
fn color_normalizes_mixed_case() {
    let c = Color::new("#Ff8800fF".to_string()).expect("mixed case must parse");
    assert_eq!(
        c.as_str(),
        "#ff8800ff",
        "mixed case input must store lowercase"
    );
}

#[test]
fn color_rejects_no_alpha() {
    // 6 hex chars (no alpha) — schema requires 8.
    assert!(Color::new("#ff8800".to_string()).is_err());
}

#[test]
fn color_rejects_3_digit_hex() {
    assert!(Color::new("#fff".to_string()).is_err());
}

#[test]
fn color_rejects_non_hex() {
    // CSS color name.
    assert!(Color::new("red".to_string()).is_err());
    // Missing leading `#`.
    assert!(Color::new("ff8800ff".to_string()).is_err());
    // 8 chars with a `g`.
    assert!(Color::new("#ff8800gg".to_string()).is_err());
}

#[test]
fn color_serde_roundtrip_normalizes() {
    // Deserialize uppercase → store lowercase → serialize lowercase.
    let c: Color =
        serde_json::from_value(json!("#FFAA00FF")).expect("uppercase JSON must parse via serde");
    assert_eq!(c.as_str(), "#ffaa00ff");
    let back = serde_json::to_value(&c).expect("Color → Value");
    assert_eq!(back, json!("#ffaa00ff"));
}
