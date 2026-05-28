//! `Capability` — pinned set of AI surfaces the engine commits to at
//! v0. Each variant corresponds to a `Research-04`-named model family.

/// AI surfaces the engine commits to expose. Future capabilities
/// (`BiRefNet` background removal, `YAMNet` audio classification, `SAM 2.1`
/// subject tracking) land when the upstream verb arc commits to the
/// per-capability arg shape — not at v0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    /// `faster-whisper` family for caption / transcription
    /// (Research 04 §4, spec §10.1 / §10.3). Engine-side default is
    /// `large-v3` with `tiny`/`base`/`small`/`medium` selectable per
    /// the `model` arg.
    Whisper,

    /// Rust-native librosa-equivalent onset-strength autocorrelation
    /// for beat detection (Research 04 §5, spec §19.1 `onset`
    /// algorithm). Pure DSP, no sidecar.
    Onset,

    /// Rust-native autocorrelation tempo + phase-lock for beat
    /// detection (Research 04 §5, spec §19.1 `tempo` algorithm).
    /// Pure DSP, no sidecar.
    Tempo,

    /// `BeatNet` CRNN + particle filter (ISMIR 2021) via Python
    /// sidecar (Research 04 §5 v1.x roadmap, spec §19.1 `beatnet`
    /// algorithm). Engine spawns sidecar via JSON-over-stdio.
    BeatNet,
}

impl Capability {
    /// Lower-case spec id (`"whisper"`, `"onset"`, `"tempo"`,
    /// `"beatnet"`). Matches the algorithm strings the
    /// `verbreel-state` verb args already enumerate.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Whisper => "whisper",
            Self::Onset => "onset",
            Self::Tempo => "tempo",
            Self::BeatNet => "beatnet",
        }
    }

    /// `true` if the capability runs via Python sidecar (Research 04
    /// §3 — engine spawns sidecar over JSON-stdio). `false` if it
    /// runs in-process via `ort` or pure Rust DSP.
    #[must_use]
    pub fn is_sidecar(&self) -> bool {
        matches!(self, Self::BeatNet)
    }
}
