//! [`Asset`] tagged-union enum + the 4 variant structs ([`VideoAsset`],
//! [`AudioAsset`], [`ImageAsset`], [`SubtitleAsset`]).
//!
//! Mirrors `spec/project-schema.json` `$defs/Asset` (`oneOf` over the
//! 4 variants) plus `$defs/VideoAsset` etc. The schema discriminates
//! variants via a `kind: const "<name>"` field; serde's
//! `#[serde(tag = "kind")]` produces and consumes that field at the
//! parent level so the variant structs **must not** carry their own
//! `kind` field.

use serde::{Deserialize, Serialize};
use verbreel_events::Timestamp;
use verbreel_types::AssetId;

use crate::asset_meta::{
    AudioAssetMetadata, ImageAssetMetadata, SubtitleAssetMetadata, VideoAssetMetadata,
};
use crate::newtypes::{AssetPath, Sha256};

/// The asset enum — a tagged union over the 4 asset variants. The
/// schema discriminator is the `kind` const on each variant; serde's
/// `#[serde(tag = "kind", rename_all = "lowercase")]` produces and
/// consumes that field at the parent level. Round-trip output shape:
///
/// ```json
/// { "kind": "video", "id": "...", "hash": "...", "path": "...", ... }
/// ```
///
/// — NOT a nested-enum wrapping like `{ "Video": { ... } }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Asset {
    /// Video asset variant. Carries [`VideoAssetMetadata`].
    Video(VideoAsset),
    /// Audio asset variant. Carries [`AudioAssetMetadata`].
    Audio(AudioAsset),
    /// Image asset variant. Carries [`ImageAssetMetadata`].
    Image(ImageAsset),
    /// Subtitle asset variant. Carries [`SubtitleAssetMetadata`].
    Subtitle(SubtitleAsset),
}

impl Asset {
    /// Borrow the asset's [`AssetId`] regardless of variant.
    #[must_use]
    pub fn id(&self) -> &AssetId {
        match self {
            Asset::Video(a) => &a.id,
            Asset::Audio(a) => &a.id,
            Asset::Image(a) => &a.id,
            Asset::Subtitle(a) => &a.id,
        }
    }

    /// Borrow the asset's [`Sha256`] hash regardless of variant.
    #[must_use]
    pub fn hash(&self) -> &Sha256 {
        match self {
            Asset::Video(a) => &a.hash,
            Asset::Audio(a) => &a.hash,
            Asset::Image(a) => &a.hash,
            Asset::Subtitle(a) => &a.hash,
        }
    }

    /// Borrow the asset's content-addressed [`AssetPath`] regardless
    /// of variant.
    #[must_use]
    pub fn path(&self) -> &AssetPath {
        match self {
            Asset::Video(a) => &a.path,
            Asset::Audio(a) => &a.path,
            Asset::Image(a) => &a.path,
            Asset::Subtitle(a) => &a.path,
        }
    }
}

// ---------------------------------------------------------------------
// VideoAsset
// ---------------------------------------------------------------------

/// Video asset record. Schema `$defs/VideoAsset`.
///
/// **No `kind` field** — serde's `#[serde(tag = "kind")]` on the
/// parent [`Asset`] enum produces it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoAsset {
    /// Asset ID.
    pub id: AssetId,
    /// SHA-256 of the imported bytes.
    pub hash: Sha256,
    /// Content-addressed disk path.
    pub path: AssetPath,
    /// Original filename at import time.
    pub original_filename: String,
    /// RFC 3339 timestamp of import. Typed via [`Timestamp`] (issue
    /// #382): validated on construction, serialized as a date-time
    /// string.
    pub imported_at: Timestamp,
    /// Codec / container / fingerprint details.
    pub metadata: VideoAssetMetadata,
}

// ---------------------------------------------------------------------
// AudioAsset
// ---------------------------------------------------------------------

/// Audio asset record. Schema `$defs/AudioAsset`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioAsset {
    /// Asset ID.
    pub id: AssetId,
    /// SHA-256 of the imported bytes.
    pub hash: Sha256,
    /// Content-addressed disk path.
    pub path: AssetPath,
    /// Original filename at import time.
    pub original_filename: String,
    /// RFC 3339 timestamp of import. Typed via [`Timestamp`] (issue #382).
    pub imported_at: Timestamp,
    /// Codec / container / fingerprint details.
    pub metadata: AudioAssetMetadata,
}

// ---------------------------------------------------------------------
// ImageAsset
// ---------------------------------------------------------------------

/// Image asset record. Schema `$defs/ImageAsset`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageAsset {
    /// Asset ID.
    pub id: AssetId,
    /// SHA-256 of the imported bytes.
    pub hash: Sha256,
    /// Content-addressed disk path.
    pub path: AssetPath,
    /// Original filename at import time.
    pub original_filename: String,
    /// RFC 3339 timestamp of import. Typed via [`Timestamp`] (issue #382).
    pub imported_at: Timestamp,
    /// Dimensions / fingerprint details.
    pub metadata: ImageAssetMetadata,
}

// ---------------------------------------------------------------------
// SubtitleAsset
// ---------------------------------------------------------------------

/// Subtitle asset record. Schema `$defs/SubtitleAsset`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubtitleAsset {
    /// Asset ID.
    pub id: AssetId,
    /// SHA-256 of the imported bytes.
    pub hash: Sha256,
    /// Content-addressed disk path.
    pub path: AssetPath,
    /// Original filename at import time.
    pub original_filename: String,
    /// RFC 3339 timestamp of import. Typed via [`Timestamp`] (issue #382).
    pub imported_at: Timestamp,
    /// Format / language / fingerprint details.
    pub metadata: SubtitleAssetMetadata,
}
