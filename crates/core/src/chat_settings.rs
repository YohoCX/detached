use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapters::CLAUDE;
use crate::backend::Model;
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
    /// The models the provider last reported.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<Model>,
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

    pub fn models(&self, provider: &str) -> &[Model] {
        self.providers.get(provider).map_or(&[], |c| &c.models)
    }

    /// Keeps `models` as the provider's list, writing only when it differs; an empty list is not one.
    pub fn remember(core: &AppState, provider: &str, models: &[Model]) -> Result<()> {
        if models.is_empty() || Self::read(&core.get_settings()).models(provider) == models {
            return Ok(());
        }
        Self::save(core, |p| p.choices(provider).models = models.to_vec()).map(drop)
    }

    pub fn save(core: &AppState, edit: impl FnOnce(&mut Self)) -> Result<Settings> {
        static SAVE: Mutex<()> = Mutex::new(());
        let _guard = SAVE.lock().unwrap();
        let mut preset = Self::read(&core.get_settings());
        edit(&mut preset);
        core.set_block(BLOCK, preset)
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

    #[test]
    fn a_reported_model_list_is_written_only_when_it_differs() {
        let sb = Sandbox::new();
        let opus = vec![Model {
            value: "opus".into(),
            label: "Opus".into(),
            efforts: vec!["low".into(), "high".into()],
            ..Model::default()
        }];
        let models = |s: &Settings| Preset::read(s).models("claude").to_vec();
        assert!(models(&sb.state.get_settings()).is_empty());
        Preset::remember(&sb.state, "claude", &opus).unwrap();
        let revision = sb.state.revision();
        Preset::remember(&sb.state, "claude", &opus).unwrap();
        Preset::remember(&sb.state, "claude", &[]).unwrap();
        assert_eq!(sb.state.revision(), revision);
        let saved = Settings::load(&sb.state.config_dir);
        assert_eq!(models(&saved), opus);
        assert!(Preset::read(&saved).models("codex").is_empty());
        let sonnet = vec![Model {
            value: "sonnet".into(),
            ..opus[0].clone()
        }];
        Preset::remember(&sb.state, "claude", &sonnet).unwrap();
        assert_ne!(sb.state.revision(), revision);
        assert_eq!(models(&sb.state.get_settings()), sonnet);
    }
}
