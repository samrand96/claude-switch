//! Terminal palettes. The active palette is process-global and set once from the config.

use std::sync::{OnceLock, RwLock};

use console::Style;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn parse(input: &str) -> Option<Rgb> {
        let hex = input.trim().trim_start_matches('#');
        if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        Some(Rgb(byte(0)?, byte(2)?, byte(4)?))
    }

    pub fn hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.0, self.1, self.2)
    }

    /// Nearest xterm-256 color, for terminals without 24-bit color.
    fn ansi256(self) -> u8 {
        let Rgb(r, g, b) = self;
        if r == g && g == b {
            if r < 8 {
                return 16;
            }
            if r > 248 {
                return 231;
            }
            return 232 + ((r as u16 - 8) * 24 / 247) as u8;
        }
        let level = |v: u8| -> u8 {
            if v < 48 {
                0
            } else if v < 115 {
                1
            } else {
                ((v - 35) / 40).min(5)
            }
        };
        16 + 36 * level(r) + 6 * level(g) + level(b)
    }

    pub fn style(self) -> Style {
        if true_color() {
            Style::new().true_color(self.0, self.1, self.2)
        } else {
            Style::new().color256(self.ansi256())
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub accent: Rgb,
    pub accent2: Rgb,
    pub success: Rgb,
    pub warning: Rgb,
    pub error: Rgb,
    pub info: Rgb,
    pub dim: Rgb,
    pub text: Rgb,
}

pub struct Theme {
    pub name: &'static str,
    pub description: &'static str,
    pub palette: Palette,
}

pub const THEMES: [Theme; 5] = [
    Theme {
        name: "claude",
        description: "Warm terracotta, the Claude look",
        palette: Palette {
            accent: Rgb(0xD9, 0x77, 0x57),
            accent2: Rgb(0xF0, 0xB4, 0x8E),
            success: Rgb(0x7F, 0xC2, 0x8A),
            warning: Rgb(0xE8, 0xB9, 0x4A),
            error: Rgb(0xF0, 0x6A, 0x5F),
            info: Rgb(0x8F, 0xB4, 0xE0),
            dim: Rgb(0x8A, 0x84, 0x7C),
            text: Rgb(0xEC, 0xE6, 0xDE),
        },
    },
    Theme {
        name: "ocean",
        description: "Cool blues and teals",
        palette: Palette {
            accent: Rgb(0x38, 0xBD, 0xF8),
            accent2: Rgb(0x81, 0xE6, 0xD9),
            success: Rgb(0x4A, 0xDE, 0x80),
            warning: Rgb(0xFB, 0xBF, 0x24),
            error: Rgb(0xF8, 0x71, 0x71),
            info: Rgb(0x93, 0xC5, 0xFD),
            dim: Rgb(0x7B, 0x8B, 0xA1),
            text: Rgb(0xE2, 0xE8, 0xF0),
        },
    },
    Theme {
        name: "matrix",
        description: "Green phosphor",
        palette: Palette {
            accent: Rgb(0x50, 0xFF, 0x91),
            accent2: Rgb(0xB6, 0xFF, 0xC9),
            success: Rgb(0x3C, 0xDC, 0x6E),
            warning: Rgb(0xBE, 0xD2, 0x5F),
            error: Rgb(0xFF, 0x5C, 0x5C),
            info: Rgb(0x60, 0xBE, 0x87),
            dim: Rgb(0x56, 0x7A, 0x60),
            text: Rgb(0xE2, 0xFF, 0xE6),
        },
    },
    Theme {
        name: "dusk",
        description: "Violet and pink",
        palette: Palette {
            accent: Rgb(0xC0, 0x84, 0xFC),
            accent2: Rgb(0xF4, 0x72, 0xB6),
            success: Rgb(0x86, 0xEF, 0xAC),
            warning: Rgb(0xFD, 0xE0, 0x47),
            error: Rgb(0xFB, 0x71, 0x85),
            info: Rgb(0xA5, 0xB4, 0xFC),
            dim: Rgb(0x8B, 0x84, 0x9E),
            text: Rgb(0xED, 0xE9, 0xFE),
        },
    },
    Theme {
        name: "light",
        description: "Darker tones for light terminal backgrounds",
        palette: Palette {
            accent: Rgb(0xB4, 0x4D, 0x2E),
            accent2: Rgb(0x8A, 0x3A, 0x22),
            success: Rgb(0x1F, 0x8A, 0x3C),
            warning: Rgb(0x9A, 0x6A, 0x00),
            error: Rgb(0xC0, 0x26, 0x26),
            info: Rgb(0x1D, 0x5F, 0xB0),
            dim: Rgb(0x6B, 0x65, 0x5E),
            text: Rgb(0x1F, 0x1B, 0x16),
        },
    },
];

static ACTIVE: OnceLock<RwLock<Palette>> = OnceLock::new();
static TRUE_COLOR: OnceLock<bool> = OnceLock::new();

pub fn set_theme(name: &str) {
    let palette = THEMES
        .iter()
        .find(|t| t.name.eq_ignore_ascii_case(name))
        .unwrap_or(&THEMES[0])
        .palette;
    let lock = ACTIVE.get_or_init(|| RwLock::new(palette));
    if let Ok(mut guard) = lock.write() {
        *guard = palette;
    }
}

pub fn palette() -> Palette {
    ACTIVE
        .get_or_init(|| RwLock::new(THEMES[0].palette))
        .read()
        .map(|g| *g)
        .unwrap_or(THEMES[0].palette)
}

fn true_color() -> bool {
    *TRUE_COLOR.get_or_init(|| {
        let colorterm = std::env::var("COLORTERM")
            .unwrap_or_default()
            .to_lowercase();
        if colorterm.contains("truecolor") || colorterm.contains("24bit") {
            return true;
        }
        if std::env::var_os("WT_SESSION").is_some() {
            return true;
        }
        match std::env::var("TERM_PROGRAM").unwrap_or_default().as_str() {
            "Apple_Terminal" => false,
            "iTerm.app" | "vscode" | "WezTerm" | "ghostty" | "Hyper" => true,
            // Windows 10+ consoles understand 24-bit sequences.
            _ => cfg!(windows),
        }
    })
}

/// Honors NO_COLOR and disables color when output is not a terminal.
pub fn init_color_support() {
    if std::env::var_os("NO_COLOR").is_some() {
        console::set_colors_enabled(false);
        console::set_colors_enabled_stderr(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_colors() {
        assert_eq!(Rgb::parse("#3B82F6"), Some(Rgb(0x3B, 0x82, 0xF6)));
        assert_eq!(Rgb::parse("d97757"), Some(Rgb(0xD9, 0x77, 0x57)));
        assert_eq!(Rgb::parse("nope"), None);
        assert_eq!(Rgb(1, 2, 3).hex(), "#010203");
    }

    #[test]
    fn approximates_256_colors_in_range() {
        for t in &THEMES {
            let _ = t.palette.accent.ansi256();
        }
        assert_eq!(Rgb(0, 0, 0).ansi256(), 16);
        assert_eq!(Rgb(255, 255, 255).ansi256(), 231);
    }
}
