//! [`TextElement`] — text-overlay rendering parameters. Mirrors
//! `spec/project-schema.json` `$defs/TextElement`.

use serde::{Deserialize, Serialize};

use crate::newtypes::Color;
use crate::shadow::Shadow;

/// Text-clip rendering parameters.
///
/// Schema reference: `spec/project-schema.json` `$defs/TextElement`.
///
/// Required: `content` (the text itself). The other 13 fields are
/// optional with defaults that match the schema; the [`Default`] impl
/// reproduces them (with `content = ""` as the convention).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextElement {
    /// Text content. Spec: ≤ 8192 chars.
    pub content: String,
    /// Font family name. Default `"Inter"`.
    #[serde(default = "default_font_family")]
    pub font_family: String,
    /// Font size in pixels. Default `64`. Spec `minimum: 1`.
    #[serde(default = "default_font_size_px")]
    pub font_size_px: f64,
    /// Font weight (CSS-style 100..=900). Default `700`.
    #[serde(default = "default_font_weight")]
    pub font_weight: u32,
    /// Italic. Default `false`.
    #[serde(default)]
    pub italic: bool,
    /// Foreground text color. Default `"#ffffffff"`.
    #[serde(default = "default_text_color")]
    pub color: Color,
    /// Optional background color (no schema default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg_color: Option<Color>,
    /// Optional stroke (outline) color (no schema default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_color: Option<Color>,
    /// Stroke (outline) width in pixels. Default `0`. Spec
    /// `minimum: 0`.
    #[serde(default)]
    pub stroke_px: f64,
    /// Horizontal alignment within the text box. Default `Center`.
    #[serde(default = "default_align")]
    pub align: TextAlign,
    /// Inter-character spacing in pixels. Default `0`.
    #[serde(default)]
    pub letter_spacing: f64,
    /// Line-height multiplier. Default `1.2`. Spec `minimum: 0.5`.
    #[serde(default = "default_line_height")]
    pub line_height: f64,
    /// Optional drop shadow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<Shadow>,
    /// Padding around the text box in pixels. Default `0`. Spec
    /// `minimum: 0`.
    #[serde(default)]
    pub padding_px: f64,
}

/// Text-box horizontal alignment. Schema `$defs/TextElement.align`
/// `enum: ["left", "center", "right"]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TextAlign {
    /// Left-align text within the text box.
    Left,
    /// Center-align (the schema default).
    #[default]
    Center,
    /// Right-align.
    Right,
}

impl Default for TextElement {
    fn default() -> Self {
        TextElement {
            content: String::new(),
            font_family: default_font_family(),
            font_size_px: 64.0,
            font_weight: 700,
            italic: false,
            color: default_text_color(),
            bg_color: None,
            stroke_color: None,
            stroke_px: 0.0,
            align: TextAlign::Center,
            letter_spacing: 0.0,
            line_height: 1.2,
            shadow: None,
            padding_px: 0.0,
        }
    }
}

fn default_font_family() -> String {
    "Inter".to_string()
}

fn default_font_size_px() -> f64 {
    64.0
}

fn default_font_weight() -> u32 {
    700
}

fn default_text_color() -> Color {
    Color::new("#ffffffff".to_string()).expect("compile-time-valid color literal")
}

fn default_align() -> TextAlign {
    TextAlign::Center
}

fn default_line_height() -> f64 {
    1.2
}
