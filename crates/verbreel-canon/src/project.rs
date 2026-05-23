//! `project_hash` per spec §0.5.2 — canonical JSON of `project.json` MINUS
//! `updated_at` and `last_saved_event_id` (save-bookkeeping fields).

use crate::jcs::{CanonError, sha256_hex};
use serde_json::Value;

/// Compute the spec §0.5.2 `project_hash` from a project JSON value.
///
/// Strips `Project.updated_at` and `Project.last_saved_event_id` before
/// canonicalizing, so that:
/// - `project.save` (which bumps these fields) does NOT invalidate cache keys
/// - `timeline.snapshot.project_hash` returns the same value before / after a save
/// - A `load → save → reload` cycle without mutation produces an identical hash
///
/// # Errors
///
/// Propagates [`CanonError`] from the underlying canonicalizer.
pub fn project_hash(project: &Value) -> Result<String, CanonError> {
    // Clone so we don't mutate caller's value; cheap for typical project sizes.
    let mut projected = project.clone();
    if let Value::Object(obj) = &mut projected {
        obj.remove("updated_at");
        obj.remove("last_saved_event_id");
    }
    sha256_hex(&projected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn project_hash_excludes_updated_at_and_last_saved_event_id() {
        let p1 = json!({
            "id": "0190b8d3-15e3-7000-b8d3-15e370a30000",
            "name": "demo",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_saved_event_id": "0190b8d3-15e3-7000-b8d3-15e370a40000",
        });
        let p2 = json!({
            "id": "0190b8d3-15e3-7000-b8d3-15e370a30000",
            "name": "demo",
            "updated_at": "2026-12-31T23:59:59Z",  // different
            "last_saved_event_id": "0190b8d3-15e3-7000-b8d3-15e370a50000",  // different
        });

        assert_eq!(
            project_hash(&p1).unwrap(),
            project_hash(&p2).unwrap(),
            "project_hash must NOT change when only updated_at/last_saved_event_id differ"
        );
    }

    #[test]
    fn project_hash_changes_when_real_field_changes() {
        let p1 = json!({"id": "x", "name": "demo"});
        let p2 = json!({"id": "x", "name": "demo-renamed"});

        assert_ne!(
            project_hash(&p1).unwrap(),
            project_hash(&p2).unwrap(),
            "project_hash MUST change when graph content changes"
        );
    }

    #[test]
    fn project_hash_is_64_lowercase_hex() {
        let p = json!({"id": "x", "name": "demo"});
        let h = project_hash(&p).unwrap();
        assert_eq!(h.len(), 64);
        assert!(
            h.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        );
    }

    #[test]
    fn project_hash_stable_across_key_order() {
        // The JCS layer sorts keys; project_hash should be insensitive to input order.
        let p1 = json!({"id": "x", "name": "demo"});
        let p2 = json!({"name": "demo", "id": "x"});
        assert_eq!(project_hash(&p1).unwrap(), project_hash(&p2).unwrap());
    }
}
