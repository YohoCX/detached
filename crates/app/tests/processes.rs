mod common;

use std::path::Path;

use common::*;
use detached_app::settings;
use detached_core::processes::{self, Agents, Definition};
use detached_core::secrets;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{TestAppContext, VisualTestContext, point, px, size};
use kernel::doc::Doc;

fn label(cx: &mut VisualTestContext, id: &str) -> String {
    let e = cx.find(id.to_string());
    e.and_then(|e| e.label().map(String::from))
        .unwrap_or_default()
}

fn wait_label(cx: &mut VisualTestContext, id: &str, want: &str) {
    wait_until(cx, |cx| label(cx, id) == want);
}

/// The app on repo `main`, current, with the Processes app shown.
fn open(cx: &mut TestAppContext, seed: impl FnOnce(&Sandbox, &Path)) -> TestApp {
    let mut app = open_app_with(cx, |sb| {
        let main = repo(sb, "main");
        seed(sb, &main);
    });
    let main = app.sb.path("repos/main").display().to_string();
    app.workspace(|w, cx| w.set_current(Some(main), cx));
    open_tab(&mut app, "bottom.Processes");
    app
}

fn core(cx: &mut VisualTestContext) -> std::sync::Arc<detached_core::state::AppState> {
    cx.update(|_, cx| kernel::core(cx).clone())
}

fn type_in(cx: &mut VisualTestContext, id: &str, text: &str) {
    cx.click(id.to_string());
    cx.input(text);
}

#[gpui_kit::test]
fn the_process_list_fits_a_narrow_split_and_keeps_its_saved_width(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| define(sb, "main", "web"));
    let repo = cx.sb.path("repos/main").display().to_string();
    cx.workspace(|w, cx| w.show(Doc::Process { repo }, cx));
    wait_until(cx, |cx| cx.find("processes.row.web").is_some());
    cx.click("processes.row.web");
    let row = cx.find("processes.row.web").unwrap().bounds();
    let grip = cx.find("grip.processes.list").unwrap().bounds().center();
    cx.update(|w, cx| w.drag(grip, grip + point(px(160.), px(0.)), cx));
    settle(cx);
    let resized = cx.find("processes.row.web").unwrap().bounds();
    assert_eq!(resized.size.width, row.size.width + px(160.));

    cx.update(|w, cx| detached_app::actions::dispatch(w, cx, "pane.splitRight"));
    let narrow = size(px(740.), px(900.));
    cx.simulate_resize(narrow);
    wait_until(cx, |cx| {
        cx.find("processes").unwrap().bounds().size.width < px(380.)
    });
    let pane = cx.find("processes").unwrap().bounds();
    let grip = cx.find("grip.processes.list").unwrap().bounds();
    let start = cx.find("processes.start").unwrap();
    assert!(grip.right() < pane.right());
    assert!(start.visible() && start.bounds().left() >= grip.right());
    let file = std::fs::read(cx.sb.state.config_dir.join("ui.json")).unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&file).unwrap();
    assert_eq!(saved["size.processes.list"], 400.);

    cx.simulate_resize(size(px(1800.), px(900.)));
    assert_eq!(
        cx.find("processes.row.web").unwrap().bounds().size.width,
        resized.size.width
    );

    cx.simulate_resize(narrow);
    let grip = cx.find("grip.processes.list").unwrap().bounds().center();
    cx.update(|w, cx| w.drag(grip, grip - point(px(32.), px(0.)), cx));
    let moved = cx.find("grip.processes.list").unwrap().bounds().center();
    assert!((grip.x - moved.x - px(32.)).abs() <= px(1.));
}

#[gpui_kit::test]
fn dragging_one_process_list_leaves_the_other_split_alone(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        define(sb, "main", "web");
        define(sb, "other", "worker");
    });
    cx.simulate_resize(size(px(2400.), px(900.)));
    let main = cx.sb.path("repos/main").display().to_string();
    let other = cx.sb.path("repos/other").display().to_string();
    cx.workspace(|w, cx| w.show(Doc::Process { repo: main }, cx));
    wait_until(cx, |cx| cx.find("processes.row.web").is_some());
    cx.update(|w, cx| detached_app::actions::dispatch(w, cx, "pane.splitRight"));
    cx.workspace(|w, cx| w.show(Doc::Process { repo: other }, cx));
    wait_until(cx, |cx| cx.find("processes.row.worker").is_some());
    let web = cx.find("processes.row.web").unwrap().bounds();
    let worker = cx.find("processes.row.worker").unwrap().bounds();
    cx.update(|w, cx| {
        w.render_frame(cx);
        let grips = w.find_all("grip.processes.list");
        assert_eq!(grips.len(), 2);
        let grip = grips
            .iter()
            .map(|g| g.bounds().center())
            .max_by_key(|p| p.x)
            .unwrap();
        w.drag(grip, grip + point(px(80.), px(0.)), cx);
    });
    assert_eq!(cx.find("processes.row.web").unwrap().bounds(), web);
    assert_eq!(
        cx.find("processes.row.worker").unwrap().bounds().size.width,
        worker.size.width + px(80.)
    );
}

fn define(sb: &Sandbox, checkout: &str, name: &str) {
    let def = Definition {
        name: name.into(),
        command: "echo up".into(),
        ..Default::default()
    };
    processes::save(&sb.state, &repo(sb, checkout), &def).unwrap();
}

#[gpui_kit::test]
fn a_defined_process_runs_to_ready_stops_and_another_shows_its_exit_and_error(
    cx: &mut TestAppContext,
) {
    let cx = &mut open(cx, |_, _| {});
    let serve = if cfg!(windows) {
        "echo listening on 3000 && ping -n 60 127.0.0.1 >NUL"
    } else {
        "echo listening on 3000 && sleep 60"
    };
    cx.click("processes.new");
    type_in(cx, "processes.name", "web");
    type_in(cx, "processes.command", serve);
    type_in(cx, "processes.ready", "listening on \\d+");
    cx.click("processes.save");
    wait_label(cx, "processes.row.web", "web · Stopped");
    cx.click("processes.start");
    wait_label(cx, "processes.state", "Running · ready");
    assert_eq!(label(cx, "processes.log").trim(), "listening on 3000");
    cx.click("processes.stop");
    cx.click("processes.new");
    wait_until(cx, |cx| !label(cx, "processes.row.web").contains("Running"));
    settle(cx);
    type_in(cx, "processes.name", "fail");
    type_in(
        cx,
        "processes.command",
        "echo Error: port 3000 in use && exit 3",
    );
    cx.click("processes.save");
    wait_label(cx, "processes.row.fail", "fail · Stopped");
    cx.click("processes.start");
    wait_label(cx, "processes.state", "Exited with 3");
    assert_eq!(label(cx, "processes.log").trim(), "Error: port 3000 in use");
    let def = processes::get(&core(cx), &cx.sb.path("repos/main"), "fail").unwrap();
    assert_eq!((def.agents, def.profile), (Agents::Off, None));
}

#[gpui_kit::test]
fn letting_agents_start_a_process_says_it_runs_code_as_the_owner_with_the_profile_s_names(
    cx: &mut TestAppContext,
) {
    let cx = &mut open(cx, |sb, main| {
        let vars = [("API_TOKEN".into(), "not-a-real-secret".into())];
        secrets::save(&sb.state, "dev", &vars).unwrap();
        let def = Definition {
            name: "web".into(),
            command: "npm run dev".into(),
            profile: Some("dev".into()),
            ..Default::default()
        };
        processes::save(&sb.state, main, &def).unwrap();
    });
    wait_until(cx, |cx| cx.find("processes.row.web").is_some());
    cx.click("processes.row.web");
    assert!(cx.find("processes.warning").is_none());
    cx.update(|w, cx| w.within("processes.agents").click(2usize, cx));
    wait_until(cx, |cx| cx.find("processes.warning").is_some());
    let said = label(cx, "processes.warning");
    assert!(
        said.contains("`npm run dev`")
            && said.contains("outside its sandbox")
            && said.contains("dev: API_TOKEN")
            && !said.contains("not-a-real-secret"),
        "{said}"
    );
    let main = cx.sb.path("repos/main");
    let def = processes::get(&core(cx), &main, "web").unwrap();
    assert_eq!(def.agents, Agents::Start);
}

#[gpui_kit::test]
fn the_env_profiles_page_keeps_values_in_the_store_and_names_in_settings(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    cx.update(|_, cx| settings::open(cx, "env"));
    settle(cx);
    cx.click("env.new");
    type_in(cx, "env.name", "dev");
    type_in(cx, "env.var.0.name", "API_TOKEN");
    type_in(cx, "env.var.0.value", "not-a-real-secret-11");
    cx.click("env.add");
    type_in(cx, "env.var.1.name", "PORT");
    type_in(cx, "env.var.1.value", "3000");
    cx.click("env.save");
    wait_label(cx, "env.profile.dev", "dev · API_TOKEN, PORT");
    cx.click("env.edit.dev");
    type_in(cx, "env.var.1.value", "4000");
    cx.click("env.save");
    wait_until(cx, |cx| cx.find("env.save").is_none());
    let values = secrets::values(&core(cx), "dev").unwrap();
    let want = [("API_TOKEN", "not-a-real-secret-11"), ("PORT", "4000")];
    assert_eq!(values, want.map(|(k, v)| (k.to_string(), v.to_string())));
    let settings = std::fs::read_to_string(cx.sb.state.config_dir.join("settings.json")).unwrap();
    assert!(!settings.contains("not-a-real-secret"), "{settings}");
    cx.click("env.remove.dev");
    wait_until(cx, |cx| cx.find("env.profile.dev").is_none());
}

#[gpui_kit::test]
fn a_run_keeps_its_stop_when_its_definition_changes_or_goes(cx: &mut TestAppContext) {
    let serve = if cfg!(windows) {
        "echo up && ping -n 60 127.0.0.1 >NUL"
    } else {
        "echo up && sleep 60"
    };
    let cx = &mut open(cx, |sb, main| {
        for name in ["web", "api"] {
            let def = Definition {
                name: name.into(),
                command: serve.into(),
                ..Default::default()
            };
            processes::save(&sb.state, main, &def).unwrap();
        }
    });
    let main = cx.sb.path("repos/main");
    let path = main.display().to_string();
    let store = detached_core::store::RepoStore::open(&core(cx), &main).unwrap();
    let file = |name: &str| store.local.join(format!("local/processes/{name}.toml"));
    for (name, change) in [("web", true), ("api", false)] {
        let row = format!("processes.row.{name}");
        wait_until(cx, |cx| cx.find(row.clone()).is_some());
        cx.click(row.clone());
        cx.click("processes.start");
        wait_label(cx, "processes.state", "Running");
        match change {
            true => std::fs::write(file(name), "command = \"echo changed\"\n").unwrap(),
            false => std::fs::remove_file(file(name)).unwrap(),
        }
        cx.update(|_, cx| {
            let defs = kernel::proc::definitions(cx, &path);
            defs.update(cx, |d, cx| d.refresh(cx));
        });
        wait_until(cx, |cx| cx.find("processes.untrusted").is_some());
        cx.click("processes.stop");
        let p = cx.update(|_, cx| kernel::proc::get(cx, &path, name).unwrap());
        wait_until(cx, |cx| cx.update(|_, cx| !p.read(cx).running()));
        assert!(!label(cx, &row).contains("Running"), "{name}");
    }
}
