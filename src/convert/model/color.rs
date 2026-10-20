// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! A small colour type standing in for `java.awt.Color`.

/// An 8-bit RGBA colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    /// Opaque colour from `0xRRGGBB` (the high byte is ignored, like `new Color(int)`).
    pub const fn from_rgb(rgb: u32) -> Color {
        Color { r: (rgb >> 16) as u8, g: (rgb >> 8) as u8, b: rgb as u8, a: 255 }
    }

    /// Colour from `0xAARRGGBB`.
    pub const fn from_argb(argb: u32) -> Color {
        Color { r: (argb >> 16) as u8, g: (argb >> 8) as u8, b: argb as u8, a: (argb >> 24) as u8 }
    }

    /// `0xRRGGBB`.
    pub const fn rgb(self) -> u32 {
        (self.r as u32) << 16 | (self.g as u32) << 8 | self.b as u32
    }

    /// `0xAARRGGBB`.
    pub const fn argb(self) -> u32 {
        (self.a as u32) << 24 | self.rgb()
    }
}

#[cfg(test)]
mod tests {
    use super::Color;

    #[test]
    fn conversions() {
        let c = Color::from_rgb(0x12_34_56);
        assert_eq!((c.r, c.g, c.b, c.a), (0x12, 0x34, 0x56, 255));
        assert_eq!(c.rgb(), 0x123456);
        assert_eq!(c.argb(), 0xff123456);
        assert_eq!(Color::from_argb(0x80ff0000).a, 0x80);
        assert_eq!(Color::from_rgb(0xff_00_00_00).rgb(), 0);
    }
}
