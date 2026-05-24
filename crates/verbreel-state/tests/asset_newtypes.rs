//! Validation tests for the regex-enforced asset newtypes
//! ([`Sha256`], [`AssetPath`], [`AssetRef`]) + [`RotationDeg`].

use serde_json::json;
use verbreel_state::asset_meta::RotationDeg;
use verbreel_state::{AssetPath, AssetRef, Sha256};
use verbreel_types::{AssetId, UuidV7};

const VALID_SHA: &str = "53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658";
const VALID_PATH: &str =
    "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.mp4";
const NIL_UUID: &str = "00000000-0000-0000-0000-000000000000";
const KNOWN_V7: &str = "0190b8d3-15e3-7000-bd00-0000000000a1";

// ---------------------------------------------------------------------
// Sha256
// ---------------------------------------------------------------------

#[test]
fn sha256_accepts_valid_lowercase_hex() {
    let s = Sha256::new(VALID_SHA.to_string()).expect("valid sha256 must parse");
    assert_eq!(s.as_str(), VALID_SHA);
}

#[test]
fn sha256_rejects_uppercase() {
    // Schema is explicitly lowercase: `^[0-9a-f]{64}$`.
    let upper = VALID_SHA.to_uppercase();
    assert!(
        Sha256::new(upper).is_err(),
        "uppercase hex must NOT validate per schema $defs/Sha256"
    );
}

#[test]
fn sha256_rejects_wrong_length() {
    let short = &VALID_SHA[..63];
    let long = format!("{VALID_SHA}f");
    assert!(Sha256::new(short.to_string()).is_err());
    assert!(Sha256::new(long).is_err());
}

#[test]
fn sha256_rejects_non_hex() {
    // 64 chars but a `g` in the middle.
    let bad = format!("{}g{}", &VALID_SHA[..30], &VALID_SHA[31..]);
    assert!(Sha256::new(bad).is_err());
}

#[test]
fn sha256_serde_roundtrip() {
    let s = Sha256::new(VALID_SHA.to_string()).unwrap();
    let j = serde_json::to_value(&s).unwrap();
    assert_eq!(j, json!(VALID_SHA));
    let back: Sha256 = serde_json::from_value(j).unwrap();
    assert_eq!(back, s);
}

// ---------------------------------------------------------------------
// AssetPath
// ---------------------------------------------------------------------

#[test]
fn asset_path_accepts_valid_layout() {
    let p = AssetPath::new(VALID_PATH.to_string()).expect("valid path must parse");
    assert_eq!(p.as_str(), VALID_PATH);
}

#[test]
fn asset_path_rejects_traversal() {
    // Path-escape attempt — the schema regex prevents this from ever
    // being a valid AssetPath at the type layer.
    assert!(AssetPath::new("assets/../etc/passwd".to_string()).is_err());
    assert!(AssetPath::new("../assets/ab/0000.mp4".to_string()).is_err());
}

#[test]
fn asset_path_rejects_uppercase_ext() {
    // Schema requires lowercase alphanumeric extension.
    let bad = "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658.MP4";
    assert!(AssetPath::new(bad.to_string()).is_err());
}

#[test]
fn asset_path_rejects_wrong_layout() {
    // Missing extension.
    assert!(
        AssetPath::new(
            "assets/53/53ed88c925907984e34d2afc4a4fcfcda94fde0ad32c7999ec46a77cee817658"
                .to_string()
        )
        .is_err()
    );
    // Wrong prefix.
    assert!(AssetPath::new(format!("media/53/{VALID_SHA}.mp4")).is_err());
    // Single-hex shard segment.
    assert!(AssetPath::new(format!("assets/5/{VALID_SHA}.mp4")).is_err());
}

#[test]
fn asset_path_serde_roundtrip() {
    let p = AssetPath::new(VALID_PATH.to_string()).unwrap();
    let j = serde_json::to_value(&p).unwrap();
    assert_eq!(j, json!(VALID_PATH));
    let back: AssetPath = serde_json::from_value(j).unwrap();
    assert_eq!(back, p);
}

// ---------------------------------------------------------------------
// AssetRef
// ---------------------------------------------------------------------

#[test]
fn asset_ref_nil_round_trip() {
    let r = AssetRef::nil();
    assert!(r.is_nil());
    assert!(r.id().is_none());

    let j = serde_json::to_value(&r).unwrap();
    assert_eq!(
        j,
        json!(NIL_UUID),
        "AssetRef::nil() must serialize as the 36-char nil UUID string"
    );
    let back: AssetRef = serde_json::from_value(j).unwrap();
    assert!(back.is_nil(), "round-tripped AssetRef must still be nil");
    assert_eq!(back, r);
}

#[test]
fn asset_ref_id_round_trip() {
    let id = AssetId::from_uuid_v7(KNOWN_V7.parse::<UuidV7>().unwrap());
    let r = AssetRef::from_id(id);
    assert!(!r.is_nil());
    assert_eq!(r.id().unwrap(), &id);

    let j = serde_json::to_value(&r).unwrap();
    assert_eq!(
        j,
        json!(KNOWN_V7),
        "AssetRef::from_id(id) must serialize as the underlying UUIDv7 string"
    );
    let back: AssetRef = serde_json::from_value(j).unwrap();
    assert_eq!(back, r);
}

#[test]
fn asset_ref_rejects_invalid_uuid() {
    let bad: Result<AssetRef, _> = serde_json::from_value(json!("not-a-uuid"));
    assert!(bad.is_err(), "non-UUID string must NOT parse as AssetRef");
}

#[test]
fn asset_ref_rejects_v4_uuid() {
    // A UUIDv4 string — recognizable v4 (version nibble `4` in third group).
    let v4 = "550e8400-e29b-41d4-a716-446655440000";
    let bad: Result<AssetRef, _> = serde_json::from_value(json!(v4));
    assert!(
        bad.is_err(),
        "UUIDv4 must NOT parse as AssetRef (spec §0.3 UUIDv7-strict)"
    );
}

// ---------------------------------------------------------------------
// RotationDeg
// ---------------------------------------------------------------------

#[test]
fn rotation_deg_accepts_0_90_180_270() {
    for &val in &[0i64, 90, 180, 270] {
        let rd: RotationDeg = serde_json::from_value(json!(val)).expect("valid rotation");
        let back = serde_json::to_value(rd).unwrap();
        assert_eq!(back, json!(val), "round-trip must preserve the integer");
    }
}

#[test]
fn rotation_deg_rejects_45() {
    let bad: Result<RotationDeg, _> = serde_json::from_value(json!(45));
    assert!(
        bad.is_err(),
        "45° is NOT in the schema's {{0, 90, 180, 270}} enum"
    );
}

#[test]
fn rotation_deg_rejects_negative_and_360() {
    assert!(serde_json::from_value::<RotationDeg>(json!(-90)).is_err());
    assert!(serde_json::from_value::<RotationDeg>(json!(360)).is_err());
}
