//! CLI theme definitions and resolution.
//!
//! Provides semantic color roles for TUI views. Built-in themes use
//! standard ANSI colors (default) or RGB for richer palettes.

use ratatui::style::Color;

use crate::config::{CliConfig, CustomThemeColors};

/// Semantic color roles used throughout the TUI.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct CliTheme {
    pub name: String,
    /// Primary UI accent (active tab, selected item label).
    pub accent: Color,
    /// Positive values: gains, price up.
    pub positive: Color,
    /// Negative values: losses, price down.
    pub negative: Color,
    /// Warning indicators.
    pub warning: Color,
    /// Informational elements (chart data lines).
    pub info: Color,
    /// De-emphasized text (status bars, help text).
    pub muted: Color,
    /// Titles and column headers.
    pub header: Color,
    /// Chart primary data line.
    pub chart_line: Color,
    /// Chart moving average overlay.
    pub chart_ma: Color,
}

/// All built-in theme names.
pub const BUILTIN_THEMES: &[&str] = &["default", "light", "solarized", "nord", "dracula"];

/// Resolve a `CliTheme` from config: look up the active theme name,
/// falling back to "default" if not found.
pub fn resolve_theme(config: &CliConfig) -> CliTheme {
    // Check custom themes first
    if let Some(custom) = config.custom_themes.get(&config.theme) {
        return apply_custom(custom, &config.theme);
    }
    // Built-in
    builtin_theme(&config.theme).unwrap_or_else(|| {
        eprintln!("Warning: unknown theme '{}', using default", config.theme);
        builtin_theme("default").unwrap()
    })
}

/// Return a built-in theme by name.
pub fn builtin_theme(name: &str) -> Option<CliTheme> {
    Some(match name {
        "default" => theme_default(),
        "light" => theme_light(),
        "solarized" => theme_solarized(),
        "nord" => theme_nord(),
        "dracula" => theme_dracula(),
        _ => return None,
    })
}

// ── Built-in themes ───────────────────────────────────────────────────────────

fn theme_default() -> CliTheme {
    CliTheme {
        name: "default".into(),
        accent: Color::Yellow,
        positive: Color::Green,
        negative: Color::Red,
        warning: Color::Yellow,
        info: Color::Cyan,
        muted: Color::DarkGray,
        header: Color::White,
        chart_line: Color::Cyan,
        chart_ma: Color::Magenta,
    }
}

fn theme_light() -> CliTheme {
    CliTheme {
        name: "light".into(),
        accent: Color::Rgb(0, 122, 204),     // bright blue
        positive: Color::Rgb(22, 163, 74),   // green-600
        negative: Color::Rgb(220, 38, 38),   // red-600
        warning: Color::Rgb(217, 119, 6),    // amber-600
        info: Color::Rgb(37, 99, 235),       // blue-600
        muted: Color::Rgb(107, 114, 128),    // gray-500
        header: Color::Rgb(17, 24, 39),      // gray-900
        chart_line: Color::Rgb(37, 99, 235), // blue-600
        chart_ma: Color::Rgb(147, 51, 234),  // purple-600
    }
}

fn theme_solarized() -> CliTheme {
    // Solarized Dark palette
    CliTheme {
        name: "solarized".into(),
        accent: Color::Rgb(181, 137, 0),      // yellow
        positive: Color::Rgb(133, 153, 0),    // green
        negative: Color::Rgb(220, 50, 47),    // red
        warning: Color::Rgb(203, 75, 22),     // orange
        info: Color::Rgb(38, 139, 210),       // blue
        muted: Color::Rgb(88, 110, 117),      // base01
        header: Color::Rgb(147, 161, 161),    // base1
        chart_line: Color::Rgb(42, 161, 152), // cyan
        chart_ma: Color::Rgb(108, 113, 196),  // violet
    }
}

fn theme_nord() -> CliTheme {
    // Nord palette (Polar Night + Frost + Aurora)
    CliTheme {
        name: "nord".into(),
        accent: Color::Rgb(136, 192, 208),     // nord8 (frost)
        positive: Color::Rgb(163, 190, 140),   // nord14 (aurora green)
        negative: Color::Rgb(191, 97, 106),    // nord11 (aurora red)
        warning: Color::Rgb(235, 203, 139),    // nord13 (aurora yellow)
        info: Color::Rgb(129, 161, 193),       // nord9 (frost)
        muted: Color::Rgb(76, 86, 106),        // nord3 (polar night)
        header: Color::Rgb(229, 233, 240),     // nord6 (snow storm)
        chart_line: Color::Rgb(136, 192, 208), // nord8
        chart_ma: Color::Rgb(180, 142, 173),   // nord15 (aurora purple)
    }
}

fn theme_dracula() -> CliTheme {
    // Dracula palette
    CliTheme {
        name: "dracula".into(),
        accent: Color::Rgb(189, 147, 249),     // purple
        positive: Color::Rgb(80, 250, 123),    // green
        negative: Color::Rgb(255, 85, 85),     // red
        warning: Color::Rgb(241, 250, 140),    // yellow
        info: Color::Rgb(139, 233, 253),       // cyan
        muted: Color::Rgb(98, 114, 164),       // comment
        header: Color::Rgb(248, 248, 242),     // foreground
        chart_line: Color::Rgb(139, 233, 253), // cyan
        chart_ma: Color::Rgb(255, 121, 198),   // pink
    }
}

// ── Custom theme resolution ───────────────────────────────────────────────────

fn apply_custom(custom: &CustomThemeColors, name: &str) -> CliTheme {
    let base = builtin_theme(&custom.base).unwrap_or_else(theme_default);
    CliTheme {
        name: name.to_string(),
        accent: parse_hex_or(custom.accent.as_ref(), base.accent),
        positive: parse_hex_or(custom.positive.as_ref(), base.positive),
        negative: parse_hex_or(custom.negative.as_ref(), base.negative),
        warning: parse_hex_or(custom.warning.as_ref(), base.warning),
        info: parse_hex_or(custom.info.as_ref(), base.info),
        muted: parse_hex_or(custom.muted.as_ref(), base.muted),
        header: parse_hex_or(custom.header.as_ref(), base.header),
        chart_line: parse_hex_or(custom.chart_line.as_ref(), base.chart_line),
        chart_ma: parse_hex_or(custom.chart_ma.as_ref(), base.chart_ma),
    }
}

/// Parse an optional hex color string like "#RRGGBB", falling back to default.
fn parse_hex_or(hex: Option<&String>, fallback: Color) -> Color {
    hex.map(String::as_str)
        .and_then(parse_hex_color)
        .unwrap_or(fallback)
}

/// Parse "#RRGGBB" into a ratatui `Color::Rgb`.
fn parse_hex_color(s: &str) -> Option<Color> {
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_color() {
        assert_eq!(parse_hex_color("#FF0000"), Some(Color::Rgb(255, 0, 0)));
        assert_eq!(parse_hex_color("00ff00"), Some(Color::Rgb(0, 255, 0)));
        assert_eq!(parse_hex_color("#abc"), None); // too short
        assert_eq!(parse_hex_color("xyz123"), None); // invalid
    }

    #[test]
    fn test_all_builtins_resolve() {
        for name in BUILTIN_THEMES {
            assert!(builtin_theme(name).is_some(), "missing builtin: {name}");
        }
    }

    #[test]
    fn test_default_config_resolves() {
        let config = CliConfig::default();
        let theme = resolve_theme(&config);
        assert_eq!(theme.name, "default");
    }

    #[test]
    fn test_custom_theme_with_overrides() {
        let config = CliConfig {
            theme: "my-theme".into(),
            custom_themes: {
                let mut m = std::collections::BTreeMap::new();
                m.insert(
                    "my-theme".into(),
                    CustomThemeColors {
                        base: "nord".into(),
                        accent: Some("#FF0000".into()),
                        positive: None,
                        negative: None,
                        warning: None,
                        info: None,
                        muted: None,
                        header: None,
                        chart_line: None,
                        chart_ma: None,
                    },
                );
                m
            },
        };
        let theme = resolve_theme(&config);
        assert_eq!(theme.name, "my-theme");
        assert_eq!(theme.accent, Color::Rgb(255, 0, 0));
        // positive should inherit from nord
        assert_eq!(theme.positive, Color::Rgb(163, 190, 140));
    }

    use crate::config::CliConfig;
}
