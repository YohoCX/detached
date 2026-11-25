use std::sync::mpsc;
use std::time::{Duration, Instant};

use detached_core::adapters::{self, Config, PROVIDERS, Provider};
use detached_core::backend::{Event, Kind, Name, Spec, Update};
use detached_core::chat_settings::Preset;
use detached_core::error::{Error, Result};
use detached_core::state::AppState;
use gpui::{App, Task};

pub fn start(cx: &App) -> Task<()> {
    let probes = PROVIDERS.map(|provider| {
        let core = crate::core(cx).clone();
        crate::blocking::run(cx, move || {
            let _ = refresh(&core, provider);
        })
    });
    cx.spawn(async move |_| {
        futures::future::join_all(probes).await;
    })
}

fn refresh(core: &AppState, provider: &'static Provider) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let settings = core.get_settings();
    let config: Config = settings.block(adapters::BLOCK).unwrap_or_default();
    if !config.shows(provider) {
        return Ok(());
    }
    let env = core.env();
    let path = config.cli.get(provider.id).map(String::as_str);
    let Some(cli) = adapters::find_cli(provider, path, &env) else {
        return Ok(());
    };
    let version = adapters::cli_version_until(&cli, &env, Some(deadline))?;
    let stamp = match provider.backend {
        Name::Direct => version.to_string(),
        Name::Acp => {
            let installed = adapters::installed(&core.data_dir, provider.adapter);
            let Some(adapter) = installed.current else {
                return Ok(());
            };
            format!("{version}:{adapter}")
        }
    };
    let preset = Preset::read(&settings);
    if preset
        .providers
        .get(provider.id)
        .is_some_and(|choices| choices.cli.as_ref() == Some(&stamp) && !choices.models.is_empty())
    {
        return Ok(());
    }

    let dir = tempfile::tempdir()?;
    let folder = dir
        .path()
        .to_str()
        .ok_or_else(|| Error::Other("Invalid temp path".into()))?;
    let mut spec = Spec::new(provider, folder, Kind::Chat);
    spec.trusted = true;
    spec.open_deadline = Some(deadline);
    let (tx, rx) = mpsc::channel();
    let session = super::backend(provider).open(
        core,
        &spec,
        Box::new(move |e| {
            if matches!(
                e,
                Event::Opened(_)
                    | Event::Update(Update::Commands(_))
                    | Event::Closed
                    | Event::Exited(_)
            ) {
                let _ = tx.send(e);
            }
        }),
    )?;
    let Ok(Event::Opened(mut opened)) =
        rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
    else {
        return Ok(());
    };
    if provider.backend == Name::Acp && opened.commands.is_none() {
        let until = deadline.min(Instant::now() + Duration::from_secs(3));
        if let Ok(Event::Update(Update::Commands(commands))) =
            rx.recv_timeout(until.saturating_duration_since(Instant::now()))
        {
            opened.commands = Some(commands);
        }
    }
    drop(session);
    if Instant::now() >= deadline {
        return Ok(());
    }
    Preset::save(core, |preset| {
        let choices = preset.choices(provider.id);
        choices.models = opened.models;
        choices.efforts = opened
            .config
            .into_iter()
            .find(|c| c.id == "effort" && choices.models.iter().all(|m| m.efforts.is_empty()))
            .map_or_else(Vec::new, |c| c.values);
        choices.commands = opened.commands.unwrap_or_default();
        choices.account = opened.account;
        choices.cli = Some(stamp);
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use detached_core::adapters::{CLAUDE, CODEX};
    use gpui::TestAppContext;

    use super::*;
    use crate::session::tests::boot_on;

    fn run(cx: &mut TestAppContext) -> Preset {
        let task = cx.read(start);
        cx.executor().allow_parking();
        cx.foreground_executor().block_test(task);
        crate::wait_until(cx, crate::settings::loaded);
        cx.read(|cx| {
            assert_eq!(crate::session::all(cx).count(), 0);
            assert!(detached_core::chat_index::index(crate::core(cx)).is_empty());
            Preset::read(crate::settings::Settings::get(cx))
        })
    }

    #[gpui::test]
    fn probes_claude_without_a_prompt_and_reuses_the_stamp(cx: &mut TestAppContext) {
        let cli = detached_core::testutil::claude::fake();
        let f = boot_on(cx, &cli, &[], &CLAUDE, |core| {
            core.change_block(adapters::BLOCK, |c: &mut Config| c.codex = false)
                .unwrap();
        });
        let config = f.dir.path().join("claude-config");
        let log = || std::fs::read_to_string(config.join("fake.log")).unwrap();
        let preset = run(cx);
        let choices = &preset.providers[CLAUDE.id];
        assert!(!choices.models.is_empty());
        assert!(choices.efforts.is_empty());
        assert!(choices.commands.iter().any(|c| c.name == "review"));
        assert_eq!(choices.account.as_ref().unwrap().plan, "Anthropic API");
        assert_eq!(log(), "initialize\n");
        assert_eq!(run(cx), preset);
        assert_eq!(log(), "initialize\n");

        std::fs::write(config.join("fake-version"), "2.1.401").unwrap();
        let preset = run(cx);
        assert_eq!(preset.providers[CLAUDE.id].cli.as_deref(), Some("2.1.401"));
        assert_eq!(log(), "initialize\ninitialize\n");
    }

    #[gpui::test]
    fn probes_codex_for_session_efforts_and_commands_without_a_prompt(cx: &mut TestAppContext) {
        let cli = detached_acp::testutil::fake();
        let home = tempfile::tempdir().unwrap();
        let log = home.path().join("fake.log");
        std::fs::write(&log, "").unwrap();
        let _f = boot_on(
            cx,
            &cli,
            &[("CODEX_HOME", home.path().to_str().unwrap())],
            &CODEX,
            |core| {
                detached_acp::testutil::install(core, "1.0.0");
                core.change_block(adapters::BLOCK, |c: &mut Config| c.claude = false)
                    .unwrap();
            },
        );
        let preset = run(cx);
        let choices = &preset.providers[CODEX.id];
        assert!(!choices.models.is_empty());
        assert_eq!(choices.efforts, ["low", "medium", "high"]);
        assert!(choices.commands.iter().any(|c| c.name == "compact"));
        assert_eq!(choices.cli.as_deref(), Some("2.1.400:1.0.0"));
        assert_eq!(std::fs::read_to_string(log).unwrap(), "");
    }
}
