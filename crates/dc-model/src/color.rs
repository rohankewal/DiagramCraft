//! Colors.
//!
//! The interchange format writes colors as hex strings and accepts the short
//! three-digit form (`#333`) alongside the six- and eight-digit forms.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::fmt;

/// A straight sRGB color with an alpha channel.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(255, 255, 255);
    pub const TRANSPARENT: Color = Color { r: 0, g: 0, b: 0, a: 0 };

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Color { r, g, b, a }
    }

    /// Parses `#rgb`, `#rgba`, `#rrggbb`, or `#rrggbbaa`. The leading `#` is
    /// optional because documents in the wild omit it.
    pub fn parse(s: &str) -> Result<Self, ColorParseError> {
        let hex = s.strip_prefix('#').unwrap_or(s);
        let nibble = |c: u8| -> Result<u8, ColorParseError> {
            (c as char)
                .to_digit(16)
                .map(|d| d as u8)
                .ok_or_else(|| ColorParseError(s.to_string()))
        };
        let bytes = hex.as_bytes();
        match bytes.len() {
            // Short form duplicates each nibble, so `#333` is `#333333`.
            3 | 4 => {
                let mut c = [255u8; 4];
                for (i, &b) in bytes.iter().enumerate() {
                    let v = nibble(b)?;
                    c[i] = v << 4 | v;
                }
                Ok(Color::rgba(c[0], c[1], c[2], c[3]))
            }
            6 | 8 => {
                let mut c = [255u8; 4];
                for (i, pair) in bytes.chunks_exact(2).enumerate() {
                    c[i] = nibble(pair[0])? << 4 | nibble(pair[1])?;
                }
                Ok(Color::rgba(c[0], c[1], c[2], c[3]))
            }
            _ => Err(ColorParseError(s.to_string())),
        }
    }

    /// Writes `#rrggbb`, widening to `#rrggbbaa` only when alpha is partial so
    /// that opaque colors survive a round-trip in their original shape.
    pub fn to_hex(self) -> String {
        if self.a == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
        }
    }

    /// Relative luminance per WCAG 2.1, used to pick a readable text color
    /// against a given fill.
    pub fn luminance(self) -> f32 {
        let channel = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * channel(self.r) + 0.7152 * channel(self.g) + 0.0722 * channel(self.b)
    }

    /// The text color the format specifies as the contrast-based default.
    pub fn contrasting_text(self) -> Color {
        if self.luminance() > 0.179 { Color::BLACK } else { Color::WHITE }
    }

    pub fn with_alpha(self, a: u8) -> Color {
        Color { a, ..self }
    }

    /// Linear interpolation in sRGB space. Adequate for UI tinting.
    pub fn lerp(self, other: Color, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Color {
            r: m(self.r, other.r),
            g: m(self.g, other.g),
            b: m(self.b, other.b),
            a: m(self.a, other.a),
        }
    }
}

impl fmt::Debug for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid color {0:?}; expected #rgb, #rgba, #rrggbb, or #rrggbbaa")]
pub struct ColorParseError(String);

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Color::parse(&s).map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_documented_form() {
        assert_eq!(Color::parse("#333").unwrap(), Color::rgb(0x33, 0x33, 0x33));
        assert_eq!(Color::parse("#800080").unwrap(), Color::rgb(0x80, 0, 0x80));
        assert_eq!(Color::parse("#32a834").unwrap(), Color::rgb(0x32, 0xa8, 0x34));
        assert_eq!(
            Color::parse("#ffffff80").unwrap(),
            Color::rgba(255, 255, 255, 0x80)
        );
        assert_eq!(Color::parse("ffffff").unwrap(), Color::WHITE);
    }

    #[test]
    fn rejects_malformed() {
        assert!(Color::parse("#12345").is_err());
        assert!(Color::parse("#zzzzzz").is_err());
        assert!(Color::parse("").is_err());
    }

    #[test]
    fn opaque_colors_round_trip_without_growing() {
        assert_eq!(Color::rgb(0x80, 0, 0x80).to_hex(), "#800080");
        assert_eq!(Color::rgba(1, 2, 3, 4).to_hex(), "#01020304");
    }

    #[test]
    fn contrast_picks_readable_text() {
        assert_eq!(Color::WHITE.contrasting_text(), Color::BLACK);
        assert_eq!(Color::BLACK.contrasting_text(), Color::WHITE);
    }
}
