//! Popup colors (straight RGBA, 0.0 to 1.0) and theme decisions.

pub type Rgba = [f32; 4];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub text: Rgba,
    pub text_secondary: Rgba,
    pub track: Rgba,
    pub button: Rgba,
    pub button_hover: Rgba,
    pub button_border: Rgba,
    /// Links (the Change link of the shortcut row).
    pub accent: Rgba,
    /// Used when the acrylic backdrop is not available.
    pub background: Rgba,
}

/// `SystemUsesLightTheme` as read from the registry. A missing value means
/// light, the Windows default.
pub fn is_light(system_uses_light_theme: Option<u32>) -> bool {
    system_uses_light_theme != Some(0)
}

/// `EnableTransparency` as read from the registry. Missing means on.
pub fn transparency_enabled(enable_transparency: Option<u32>) -> bool {
    enable_transparency != Some(0)
}

pub fn palette(light: bool) -> Palette {
    // Close to the Windows 11 flyout colors.
    if light {
        Palette {
            text: rgb(0x1B, 0x1B, 0x1B, 1.0),
            text_secondary: rgb(0x5F, 0x5F, 0x5F, 1.0),
            track: rgb(0, 0, 0, 0.10),
            button: rgb(0xFF, 0xFF, 0xFF, 0.70),
            button_hover: rgb(0xF9, 0xF9, 0xF9, 0.50),
            button_border: rgb(0, 0, 0, 0.10),
            accent: rgb(0x00, 0x5F, 0xB8, 1.0),
            background: rgb(0xF3, 0xF3, 0xF3, 1.0),
        }
    } else {
        Palette {
            text: rgb(0xFF, 0xFF, 0xFF, 1.0),
            text_secondary: rgb(0xC5, 0xC5, 0xC5, 1.0),
            track: rgb(0xFF, 0xFF, 0xFF, 0.14),
            button: rgb(0xFF, 0xFF, 0xFF, 0.06),
            button_hover: rgb(0xFF, 0xFF, 0xFF, 0.09),
            button_border: rgb(0xFF, 0xFF, 0xFF, 0.09),
            accent: rgb(0x60, 0xCD, 0xFF, 1.0),
            background: rgb(0x2C, 0x2C, 0x2C, 1.0),
        }
    }
}

const fn rgb(r: u8, g: u8, b: u8, a: f32) -> Rgba {
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luma(c: Rgba) -> f32 {
        0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
    }

    #[test]
    fn light_theme_from_registry() {
        assert!(is_light(None));
        assert!(is_light(Some(1)));
        assert!(!is_light(Some(0)));
    }

    #[test]
    fn transparency_from_registry() {
        assert!(transparency_enabled(None));
        assert!(transparency_enabled(Some(1)));
        assert!(!transparency_enabled(Some(0)));
    }

    #[test]
    fn text_contrasts_with_background() {
        for light in [true, false] {
            let p = palette(light);
            let diff = (luma(p.text) - luma(p.background)).abs();
            assert!(diff > 0.7, "light={light} contrast {diff}");
            let diff2 = (luma(p.text_secondary) - luma(p.background)).abs();
            assert!(diff2 > 0.35, "light={light} secondary contrast {diff2}");
        }
    }

    #[test]
    fn light_and_dark_differ() {
        assert!(luma(palette(true).background) > luma(palette(false).background));
        assert!(luma(palette(true).text) < luma(palette(false).text));
    }
}
