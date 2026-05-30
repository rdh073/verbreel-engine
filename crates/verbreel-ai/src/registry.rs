//! Provider registry — the parallel capability surface for AI inference.
//!
//! Reports the three algorithm classes the inference runtime can serve:
//! tracker algorithms, caption (STT) models, and audio-analysis algorithms.
//!
//! ## Why this is a *parallel* surface, not the `list_capabilities` verb
//!
//! `verbreel_state::verbs::list_capabilities` is frozen at the v1.0 floor:
//! its seven v1.1+ fields (`tracker_algorithms`, `caption_models`, …) are
//! omitted entirely so the verb's output — and the conformance fixture that
//! pins it — stays byte-stable. `verbreel-ai` depends on `verbreel-state`
//! (not the other way round), so the verb cannot call into this registry
//! without inverting the dependency edge and breaking the conformance
//! fixture.
//!
//! ## No composition-root consumer in this slice
//!
//! The intended consumer is the composition layer (cli / mcp / http), which
//! sits *above* both `verbreel-ai` and `verbreel-state` and is the one place
//! allowed to import this crate (the decision packet pins "report into
//! `list_capabilities` *without importing UI or transport crates*", so the
//! fold must happen in the higher crate, not here). Those crates are still
//! at their v1 floor and do not yet expose a `capabilities` surface, so this
//! registry currently has **no caller outside this crate** — wiring it in is
//! a deferred follow-up slice in cli / mcp / http, out of scope for the
//! single-crate `verbreel-ai` change. It is shipped now so that follow-up is
//! an additive `ProviderRegistry::v1()` read, not an API redesign.

/// A single algorithm/model the inference runtime advertises, tagged with
/// where it runs (in-process via `ort`/DSP, or out-of-process via sidecar).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderEntry {
    /// Spec algorithm/model literal (e.g. `"mixformer_v2_s"`, `"whisper"`).
    pub id: String,
    /// Human-readable backing-model description.
    pub backend: String,
    /// `true` if the entry runs out-of-process via the Python sidecar.
    pub sidecar: bool,
}

impl ProviderEntry {
    fn native(id: &str, backend: &str) -> Self {
        Self {
            id: id.to_string(),
            backend: backend.to_string(),
            sidecar: false,
        }
    }

    fn sidecar(id: &str, backend: &str) -> Self {
        Self {
            id: id.to_string(),
            backend: backend.to_string(),
            sidecar: true,
        }
    }
}

/// The three algorithm classes the inference runtime serves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRegistry {
    /// Tracker algorithms (Research 04 §3 — `MixFormerV2-S` / `YuNet` / LK).
    pub tracker_algorithms: Vec<ProviderEntry>,
    /// Caption / STT models (Research 04 §4 — faster-whisper family).
    pub caption_models: Vec<ProviderEntry>,
    /// Audio-analysis algorithms (Research 04 §5 — onset / tempo / `BeatNet`).
    pub audio_analysis_algorithms: Vec<ProviderEntry>,
}

impl ProviderRegistry {
    /// Build the v1 registry snapshot.
    ///
    /// The contents mirror the algorithm/model literals the `verbreel-state`
    /// verb args already enumerate, so the composition layer can map an
    /// engine capability id straight onto a verb's accepted `algorithm` /
    /// `model` value with no translation table.
    #[must_use]
    pub fn v1() -> Self {
        Self {
            tracker_algorithms: vec![
                ProviderEntry::native("mixformer_v2_s", "MixFormerV2-S (ort)"),
                ProviderEntry::native("yunet", "YuNet face detector (ort)"),
                ProviderEntry::native("lk", "OpenCV Lucas-Kanade optical flow"),
            ],
            caption_models: vec![ProviderEntry::sidecar(
                "whisper",
                "faster-whisper (Python sidecar)",
            )],
            audio_analysis_algorithms: vec![
                ProviderEntry::native("onset", "onset-strength autocorrelation (DSP)"),
                ProviderEntry::native("tempo", "autocorrelation tempo + phase-lock (DSP)"),
                ProviderEntry::sidecar("beatnet", "BeatNet CRNN (Python sidecar)"),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_all_three_algorithm_classes() {
        let reg = ProviderRegistry::v1();
        assert!(
            !reg.tracker_algorithms.is_empty(),
            "tracker algorithms class must be non-empty"
        );
        assert!(
            !reg.caption_models.is_empty(),
            "caption models class must be non-empty"
        );
        assert!(
            !reg.audio_analysis_algorithms.is_empty(),
            "audio-analysis algorithms class must be non-empty"
        );
    }

    #[test]
    fn tracker_ids_match_research_04_section_3() {
        let reg = ProviderRegistry::v1();
        let ids: Vec<&str> = reg
            .tracker_algorithms
            .iter()
            .map(|e| e.id.as_str())
            .collect();
        assert_eq!(ids, ["mixformer_v2_s", "yunet", "lk"]);
    }

    #[test]
    fn whisper_is_a_sidecar_caption_model() {
        let reg = ProviderRegistry::v1();
        let whisper = reg
            .caption_models
            .iter()
            .find(|e| e.id == "whisper")
            .expect("whisper must be advertised as a caption model");
        assert!(whisper.sidecar, "whisper runs via the Python sidecar");
    }

    #[test]
    fn audio_analysis_advertises_onset_tempo_beatnet() {
        let reg = ProviderRegistry::v1();
        let ids: Vec<&str> = reg
            .audio_analysis_algorithms
            .iter()
            .map(|e| e.id.as_str())
            .collect();
        assert_eq!(ids, ["onset", "tempo", "beatnet"]);
    }
}
