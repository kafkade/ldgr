//! `ldgr config` — CLI configuration management.

use anyhow::{Result, bail};

use crate::config::{self, CliConfig, save_config};
use crate::theme::BUILTIN_THEMES;

/// Set a config key to a value.
pub fn run_set(key: &str, value: &str) -> Result<()> {
    let mut config = config::load_config();

    match key {
        "theme" => {
            validate_theme_name(value, &config)?;
            config.theme = value.to_string();
            save_config(&config)?;
            eprintln!("✓ Theme set to '{value}'");
        }
        _ => bail!("Unknown config key: '{key}'. Available keys: theme"),
    }

    Ok(())
}

/// Get a config key's current value.
pub fn run_get(key: &str) -> Result<()> {
    let config = config::load_config();

    match key {
        "theme" => println!("{}", config.theme),
        _ => bail!("Unknown config key: '{key}'. Available keys: theme"),
    }

    Ok(())
}

/// List available themes (built-in + custom).
#[allow(clippy::unnecessary_wraps)]
pub fn run_list_themes() -> Result<()> {
    let config = config::load_config();

    eprintln!("Built-in themes:");
    for name in BUILTIN_THEMES {
        let marker = if *name == config.theme {
            " (active)"
        } else {
            ""
        };
        eprintln!("  {name}{marker}");
    }

    if !config.custom_themes.is_empty() {
        eprintln!();
        eprintln!("Custom themes:");
        for (name, def) in &config.custom_themes {
            let marker = if *name == config.theme {
                " (active)"
            } else {
                ""
            };
            eprintln!("  {name} (base: {}){marker}", def.base);
        }
    }

    Ok(())
}

fn validate_theme_name(name: &str, config: &CliConfig) -> Result<()> {
    if BUILTIN_THEMES.contains(&name) || config.custom_themes.contains_key(name) {
        return Ok(());
    }
    bail!(
        "Unknown theme '{name}'.\n\
         Built-in: {}.\n\
         Define custom themes in ~/.ldgr/config.json under \"custom_themes\".",
        BUILTIN_THEMES.join(", ")
    );
}
