//! `caption.edit` (§10.2) — aliasing wrapper for `text.edit` (§7.2).
//!
//! ## Spec quote (`spec/commands/caption.md` §10.2, §7.2 aliasing)
//!
//! > Alias for `text.edit` for discoverability, using the exact same
//! > args, return shape, and error semantics.

use json_patch::Patch;
use serde_json::Value;

use crate::project::Project;
use crate::reconstructor::{ReconstructError, Verb, VerbError};

pub use super::text_edit::{MAX_CONTENT_LEN, TextEditError, W_NOOP_CODE};
pub use super::text_edit::{
    TextEditArgs, TextEditData, compute_patch, data_envelope_from_post_state,
};

#[derive(Debug, Default)]
/// Wrapper verb that keeps `text.edit` semantics but advertises the
/// alternate alias id `caption.edit`.
pub struct CaptionEditVerb;

impl Verb for CaptionEditVerb {
    fn verb(&self) -> &'static str {
        "caption.edit"
    }

    fn compute_patch(
        &self,
        prior: &Project,
        args: &Value,
    ) -> Result<(Patch, Value, Vec<Value>), VerbError> {
        let text_verb = crate::verbs::text_edit::TextEditVerb;
        text_verb.compute_patch(prior, args)
    }

    fn reconstruct(
        &self,
        args: &Value,
        patch: &Value,
        warnings: &[Value],
        post_state: &Project,
    ) -> Result<Value, ReconstructError> {
        let text_verb = crate::verbs::text_edit::TextEditVerb;
        text_verb.reconstruct(args, patch, warnings, post_state)
    }
}
