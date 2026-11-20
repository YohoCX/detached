use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapters::CLAUDE;
use crate::error::Result;
use crate::settings::Settings;
use crate::state::AppState;

pub const BLOCK: &str = "chat";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preset {
    pub provider: String,
    pub providers: BTreeMap<String, Choices>,
    #[serde(flatten)]
    other: BTreeMap<String, Value>,
}

impl Default for Preset {
    fn default() -> Self {
        Self {
            provider: CLAUDE.id.into(),
            providers: BTreeMap::new(),
            other: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Choices {
    pub model: Option<String>,
    pub effort: Option<String>,
    pub mode: Option<String>,
    #[serde(flatten)]
    other: BTreeMap<String, Value>,
}

impl Preset {
    pub fn read(settings: &Settings) -> Self {
        settings.block(BLOCK).unwrap_or_default()
    }

    pub fn choices(&mut self, provider: &str) -> &mut Choices {
        self.providers.entry(provider.into()).or_default()
    }

    pub fn save(core: &AppState, edit: impl FnOnce(&mut Self)) -> Result<Settings> {
        core.set_block_with(BLOCK, edit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;
    use serde_json::json;

    #[test]
    fn the_chat_block_keeps_unknown_settings_and_provider_choices() {
        let sb = Sandbox::new();
        assert_eq!(Preset::read(&sb.state.get_settings()).provider, "claude");
        sb.state
            .set_block(
                BLOCK,
                json!({"future": true, "providers": {
                    "claude": {"model": "sonnet", "future": 7},
                    "codex": {"mode": "agent"}
                }}),
            )
            .unwrap();
        Preset::save(&sb.state, |p| {
            p.choices("claude").effort = Some("high".into())
        })
        .unwrap();
        let saved = Settings::load(&sb.state.config_dir);
        let preset = Preset::read(&saved);
        assert_eq!(preset.providers["claude"].model.as_deref(), Some("sonnet"));
        assert_eq!(preset.providers["claude"].effort.as_deref(), Some("high"));
        assert_eq!(preset.providers["claude"].mode, None);
        assert_eq!(preset.providers["codex"].mode.as_deref(), Some("agent"));
        let raw: Value = saved.block(BLOCK).unwrap();
        assert_eq!(raw["future"], true);
        assert_eq!(raw["providers"]["claude"]["future"], 7);
    }
}
