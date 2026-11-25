use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize, de::value::MapDeserializer};
use serde_json::Value;

use crate::adapters::CLAUDE;
use crate::backend::{Account, Model, SlashCommand};
use crate::error::{Error, Result};
use crate::settings::Settings;
use crate::state::AppState;

pub const BLOCK: &str = "chat";
static SAVE: Mutex<()> = Mutex::new(());

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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<Model>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub efforts: Vec<String>,
    /// Commands reported outside any project.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commands: Option<Vec<SlashCommand>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<Account>,
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

    pub fn efforts(&self, provider: &str) -> &[String] {
        self.providers.get(provider).map_or(&[], |c| &c.efforts)
    }

    /// Nonempty models replace models and efforts. For account-only updates, pass empty models and `None` for efforts.
    pub fn remember(
        core: &AppState,
        provider: &str,
        models: Vec<Model>,
        efforts: Option<Vec<String>>,
        account: Option<Account>,
    ) -> Result<()> {
        let _guard = SAVE.lock().map_err(|e| Error::Other(e.to_string()))?;
        let has_models = !models.is_empty();
        let edit = |cached: &mut Choices| {
            if has_models {
                cached.models.clone_from(&models);
            }
            if has_models || efforts.is_some() {
                cached.efforts = efforts
                    .as_ref()
                    .filter(|_| cached.models.iter().all(|m| m.efforts.is_empty()))
                    .cloned()
                    .unwrap_or_default();
            }
            if account.is_some() {
                cached.account.clone_from(&account);
            }
        };
        let mut preset = Self::read(&core.get_settings());
        let cached = preset.choices(provider);
        let previous = cached.clone();
        edit(cached);
        if *cached != previous {
            Self::edit(core, false, |p| edit(p.choices(provider)))?;
        }
        Ok(())
    }

    pub fn save(core: &AppState, edit: impl FnOnce(&mut Self)) -> Result<Settings> {
        let _guard = SAVE.lock().map_err(|e| Error::Other(e.to_string()))?;
        Self::edit(core, true, edit)
    }

    pub fn save_cache(core: &AppState, edit: impl FnOnce(&mut Self)) -> Result<Settings> {
        let _guard = SAVE.lock().map_err(|e| Error::Other(e.to_string()))?;
        Self::edit(core, false, edit)
    }

    fn edit(core: &AppState, recover: bool, edit: impl FnOnce(&mut Self)) -> Result<Settings> {
        core.edit_block(BLOCK, |raw| {
            let mut preset = match raw.map(Self::deserialize).transpose() {
                Ok(preset) => preset.unwrap_or_default(),
                Err(_) if recover => raw.map_or_else(Self::default, Self::recover),
                Err(error) => return Err(error.into()),
            };
            edit(&mut preset);
            Ok(serde_json::to_value(preset)?)
        })
    }

    fn recover(raw: &Value) -> Self {
        let Some(mut fields) = raw.as_object().cloned() else {
            return Self::default();
        };
        if fields.get("provider").is_some_and(|v| !v.is_string()) {
            fields.remove("provider");
        }
        if let Some(providers) = fields.get_mut("providers").and_then(Value::as_object_mut) {
            providers.retain(|_, choices| {
                let Some(choices) = choices.as_object_mut() else {
                    return false;
                };
                choices.retain(|key, value| {
                    Choices::deserialize(MapDeserializer::new(std::iter::once((
                        key.as_str(),
                        &*value,
                    ))))
                    .is_ok()
                });
                true
            });
        } else {
            fields.remove("providers");
        }
        Self::deserialize(Value::Object(fields)).unwrap_or_default()
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
        let claude = &preset.providers["claude"];
        assert!(claude.efforts.is_empty() && claude.commands.is_none());
        assert!(claude.cli.is_none() && claude.account.is_none());
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
        Preset::remember(&sb.state, "claude", opus.clone(), None, None).unwrap();
        let revision = sb.state.revision();
        Preset::remember(&sb.state, "claude", opus.clone(), None, None).unwrap();
        Preset::remember(&sb.state, "claude", vec![], None, None).unwrap();
        assert_eq!(sb.state.revision(), revision);
        let saved = Settings::load(&sb.state.config_dir);
        assert_eq!(models(&saved), opus);
        assert!(Preset::read(&saved).models("codex").is_empty());
        let sonnet = vec![Model {
            value: "sonnet".into(),
            ..opus[0].clone()
        }];
        Preset::remember(&sb.state, "claude", sonnet.clone(), None, None).unwrap();
        assert_ne!(sb.state.revision(), revision);
        assert_eq!(models(&sb.state.get_settings()), sonnet);
    }

    #[test]
    fn cache_writes_keep_a_malformed_chat_block_intact() {
        let sb = Sandbox::new();
        let raw = json!({"provider": "codex", "future": true, "providers": {
            "codex": {"mode": "agent"}, "claude": {"model": "sonnet", "effort": 7}
        }});
        sb.state.set_block(BLOCK, &raw).unwrap();
        let revision = sb.state.revision();
        let model = Model {
            value: "opus".into(),
            ..Model::default()
        };
        assert!(
            Preset::save_cache(&sb.state, |p| p
                .choices("claude")
                .models
                .push(model.clone()))
            .is_err()
        );
        assert!(Preset::remember(&sb.state, "claude", vec![model], None, None).is_err());
        assert_eq!(sb.state.revision(), revision);
        assert_eq!(
            Settings::load(&sb.state.config_dir).block::<Value>(BLOCK),
            Some(raw)
        );
    }

    #[test]
    fn saving_chat_preferences_defaults_only_the_malformed_fields() {
        let sb = Sandbox::new();
        sb.state
            .set_block(
                BLOCK,
                json!({"provider": "codex", "future": true, "providers": {
                    "claude": {"model": "sonnet", "effort": 7, "future": {"number": 42}},
                    "codex": {"mode": "agent"}, "broken": false
                }}),
            )
            .unwrap();
        Preset::save(&sb.state, |p| {
            p.choices("claude").mode = Some("plan".into())
        })
        .unwrap();
        let saved = Settings::load(&sb.state.config_dir);
        let preset = saved.block::<Preset>(BLOCK).unwrap();
        assert_eq!(preset.provider, "codex");
        assert_eq!(preset.providers["claude"].model.as_deref(), Some("sonnet"));
        assert_eq!(preset.providers["claude"].effort, None);
        assert_eq!(preset.providers["claude"].mode.as_deref(), Some("plan"));
        assert_eq!(preset.providers["codex"].mode.as_deref(), Some("agent"));
        let raw: Value = saved.block(BLOCK).unwrap();
        assert_eq!(raw["future"], true);
        assert_eq!(raw["providers"]["claude"]["future"], json!({"number": 42}));
    }

    #[test]
    fn session_efforts_are_cached_only_if_every_model_lacks_its_own() {
        let sb = Sandbox::new();
        let plain = Model {
            value: "plain".into(),
            ..Model::default()
        };
        let tuned = Model {
            value: "tuned".into(),
            efforts: vec!["low".into()],
            ..Model::default()
        };
        let efforts = Some(vec!["high".into()]);
        Preset::remember(
            &sb.state,
            "codex",
            vec![plain.clone()],
            efforts.clone(),
            None,
        )
        .unwrap();
        let saved = || Settings::load(&sb.state.config_dir);
        assert_eq!(Preset::read(&saved()).efforts("codex"), ["high"]);
        Preset::remember(&sb.state, "codex", vec![plain, tuned], efforts, None).unwrap();
        let raw: Value = saved().block(BLOCK).unwrap();
        assert!(raw["providers"]["codex"].get("efforts").is_none());
    }

    #[test]
    fn an_account_only_report_keeps_the_cached_models_and_efforts() {
        let sb = Sandbox::new();
        let models = vec![Model {
            value: "plain".into(),
            ..Model::default()
        }];
        Preset::remember(
            &sb.state,
            "codex",
            models.clone(),
            Some(vec!["high".into()]),
            None,
        )
        .unwrap();
        let account = Account {
            plan: "Pro".into(),
            email: None,
        };
        Preset::remember(&sb.state, "codex", vec![], None, Some(account.clone())).unwrap();
        let preset = Preset::read(&Settings::load(&sb.state.config_dir));
        assert_eq!(preset.models("codex"), models);
        assert_eq!(preset.efforts("codex"), ["high"]);
        assert_eq!(preset.providers["codex"].account.as_ref(), Some(&account));

        Preset::remember(&sb.state, "codex", models, None, None).unwrap();
        let preset = Preset::read(&Settings::load(&sb.state.config_dir));
        assert!(preset.efforts("codex").is_empty());
        assert_eq!(preset.providers["codex"].account, Some(account));
    }
}
