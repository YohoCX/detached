use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use detached_core::adapters::{self, Config, PROVIDERS, Provider};
use detached_core::backend::{Event, Kind, Name, Spec, Update, probe_dir};
use detached_core::chat_settings::Preset;
use detached_core::error::{Error, Result};
use detached_core::state::AppState;
use gpui::{App, Task};

pub fn start(cx: &App) -> Task<()> {
    let probes = PROVIDERS.map(|provider| start_provider(cx, provider));
    cx.spawn(async move |_| {
        futures::future::join_all(probes).await;
    })
}

pub(crate) fn start_provider(cx: &App, provider: &'static Provider) -> Task<()> {
    let core = crate::core(cx).clone();
    crate::blocking::run(cx, move || {
        let _ = refresh(&core, provider);
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
    let adapter = || adapters::installed(&core.data_dir, provider.adapter).current;
    let installed = match provider.backend {
        Name::Direct => None,
        Name::Acp => match adapter() {
            Some(version) => Some(version),
            None => return Ok(()),
        },
    };
    let version = adapters::cli_version_until(provider, &cli, &env, Some(deadline))?;
    let stamp = installed.as_ref().map_or_else(
        || version.to_string(),
        |adapter| format!("{version}:{adapter}"),
    );
    let preset = Preset::read(&core.get_settings());
    if preset
        .providers
        .get(provider.id)
        .is_some_and(|choices| choices.cli.as_ref() == Some(&stamp) && !choices.models.is_empty())
    {
        return Ok(());
    }

    let dir = probe_dir()?;
    let folder = dir
        .path()
        .to_str()
        .ok_or_else(|| Error::Other("Invalid temp path".into()))?;
    let mut spec = Spec::new(provider, folder, Kind::Chat);
    spec.trusted = true;
    spec.open_deadline = Some(deadline);
    spec.temporary_dir = Some(Arc::new(dir));
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
    Preset::save_cache(core, |preset| {
        if installed.is_some() && adapter() != installed {
            return;
        }
        let choices = preset.choices(provider.id);
        let complete = !opened.models.is_empty() && opened.commands.is_some();
        if !opened.models.is_empty() {
            choices.models = opened.models;
        }
        if let Some(effort) = opened.config.into_iter().find(|c| c.id == "effort")
            && choices.models.iter().all(|m| m.efforts.is_empty())
        {
            choices.efforts = effort.values;
        }
        if let Some(account) = opened.account {
            choices.account = Some(account);
        }
        if let Some(commands) = opened.commands {
            choices.commands = Some(commands);
        } else if choices.commands.is_none() && choices.cli.is_some() {
            // Older stamps also marked reports with zero commands.
            choices.commands = Some(Vec::new());
        }
        choices.cli = complete.then_some(stamp);
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use detached_core::adapters::{CLAUDE, CODEX};
    use detached_core::backend::Account;
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
        assert!(
            choices
                .commands
                .as_ref()
                .unwrap()
                .iter()
                .any(|c| c.name == "review")
        );
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
        let f = boot_on(
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
        assert!(
            choices
                .commands
                .as_ref()
                .unwrap()
                .iter()
                .any(|c| c.name == "compact")
        );
        assert_eq!(choices.cli.as_deref(), Some("2.1.400:1.0.0"));

        let account = Account {
            plan: "Pro".into(),
            email: None,
        };
        Preset::save(&f.core, |p| {
            p.choices(CODEX.id).account = Some(account.clone())
        })
        .unwrap();
        let report = home.path().join("fake-commands");
        let no_config = home.path().join("fake-no-config");
        detached_acp::testutil::install(&f.core, "1.0.1");
        std::fs::write(&report, "missing").unwrap();
        std::fs::write(&no_config, "").unwrap();
        let partial = run(cx);
        let partial = &partial.providers[CODEX.id];
        assert_eq!(partial.models, choices.models);
        assert_eq!(partial.efforts, choices.efforts);
        assert_eq!(partial.commands, choices.commands);
        assert_eq!(partial.account.as_ref(), Some(&account));
        assert!(partial.cli.is_none());

        std::fs::write(report, "empty").unwrap();
        let partial = run(cx);
        let partial = &partial.providers[CODEX.id];
        assert_eq!(partial.commands, Some(vec![]));
        assert!(partial.cli.is_none());

        std::fs::remove_file(no_config).unwrap();
        let retried = run(cx);
        let retried = &retried.providers[CODEX.id];
        assert_eq!(retried.commands, Some(vec![]));
        assert_eq!(retried.cli.as_deref(), Some("2.1.400:1.0.1"));
        assert_eq!(std::fs::read_to_string(log).unwrap(), "");
    }
}
