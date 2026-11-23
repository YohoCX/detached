//! The app for an agent to drive: `DRIVE=<dir> [REPOS=<n>] cargo test -p detached-app --test drive`
//! (`make drive`) seeds a sandbox with n repos (3), each with a diff, and the fake Claude, opens
//! the app over it in a headless window, and answers one command per connection on `<dir>/sock`
//! with one JSON line; shots go to `<dir>`. Without DRIVE it drives itself once, as the test.
//! macOS only, on the main thread its platform needs.

mod common;

fn main() {
    #[cfg(target_os = "macos")]
    drive::main();
}

#[cfg(target_os = "macos")]
mod drive {
    use std::fs::{self, Permissions};
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use crate::common;
    use detached_app::ui::theme::{self, Mode};
    use detached_core::api;
    use detached_core::git::testutil::Sandbox;
    use gpui_kit::test::{ElementSnapshot, TestEventExt, TestQueryExt, TestWindowExt};
    use gpui_kit::{
        AnyWindowHandle, App, ElementId, HeadlessAppContext, InputEvent, Keystroke, MouseButton,
        ScrollDelta, ScrollEvent, Window, point, px, size,
    };
    use serde_json::{Value, json};

    pub fn main() {
        let Some(dir) = std::env::var_os("DRIVE").map(PathBuf::from) else {
            return selftest();
        };
        let repos = std::env::var("REPOS").map_or(3, |n| n.parse().expect("REPOS is a count"));
        let listener = listen(&dir);
        println!("ready {}", dir.join("sock").display());
        serve(listener, &dir, repos);
    }

    fn listen(dir: &Path) -> UnixListener {
        fs::create_dir_all(dir).unwrap();
        let sock = dir.join("sock");
        let _ = fs::remove_file(&sock);
        let listener = UnixListener::bind(&sock)
            .expect("binds the socket, whose path takes 103 bytes at most");
        fs::set_permissions(&sock, Permissions::from_mode(0o600)).unwrap();
        listener
    }

    /// Connections queue in `listener` until the app has loaded its repos.
    fn serve(listener: UnixListener, dir: &Path, repos: usize) {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                if tx.send(stream).is_err() {
                    break;
                }
            }
        });
        let sb = Sandbox::new();
        seed(&sb, repos);
        let (cx, window) = common::headless(&sb, |_| {});
        let mut d = Driver { cx, window, dir };
        loop {
            let Ok(stream) = rx.try_recv() else {
                common::tick(&mut d.cx);
                continue;
            };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut line = String::new();
            if BufReader::new(&stream).read_line(&mut line).is_err() {
                continue;
            }
            let line = line.trim_end_matches(['\r', '\n']);
            let reply = match d.run(line) {
                Ok(v) => json!({ "ok": v }),
                Err(e) => json!({ "error": e }),
            };
            let _ = writeln!(&stream, "{reply}");
            if line == "quit" {
                break;
            }
        }
        let _ = fs::remove_file(dir.join("sock"));
    }

    fn seed(sb: &Sandbox, repos: usize) {
        let folder = sb.path("repos");
        fs::create_dir_all(&folder).unwrap();
        api::add_folder(&sb.state, None, folder.display().to_string()).unwrap();
        for i in 1..=repos {
            let repo = common::repo(sb, &format!("repo-{i}"));
            fs::write(repo.join("README.md"), "hello\nchanged\n").unwrap();
        }
        if repos > 0 {
            let main = folder.join("repo-1");
            let tree = sb.path("renewal");
            sb.git(
                &main,
                &[
                    "worktree",
                    "add",
                    "-b",
                    "feature/renewal",
                    &tree.display().to_string(),
                ],
            );
            fs::write(tree.join("renewal.txt"), "Notes for the renewal.\n").unwrap();
        }
        common::claude(sb, &detached_core::testutil::claude::fake());
    }

    struct Driver<'a> {
        cx: HeadlessAppContext,
        window: AnyWindowHandle,
        dir: &'a Path,
    }

    impl Driver<'_> {
        /// Runs `f` on a fresh frame, then lets the app run until it is idle.
        fn with<R>(&mut self, f: impl FnOnce(&mut Window, &mut App) -> R) -> Result<R, String> {
            let r = self.cx.update_window(self.window, |_, w, cx| {
                w.render_frame(cx);
                f(w, cx)
            });
            self.cx.run_until_parked();
            r.map_err(|e| e.to_string())
        }

        fn run(&mut self, line: &str) -> Result<Value, String> {
            let (cmd, arg) = line.split_once(' ').unwrap_or((line, ""));
            match cmd {
                "shot" => {
                    let path = self.dir.join(format!("{arg}.png"));
                    let img = self.with(|w, _| w.render_to_image())?;
                    img.map_err(|e| e.to_string())?
                        .save(&path)
                        .map_err(|e| e.to_string())?;
                    Ok(json!(path))
                }
                "tree" => self.with(|w, _| {
                    w.elements()
                        .iter()
                        .filter(|e| e.visible())
                        .map(node)
                        .collect()
                }),
                "click" => {
                    let at = self.with(|w, _| find(w, arg))??.bounds().center();
                    self.with(|w, cx| {
                        w.pointer_move(at, None, cx);
                        w.pointer_down(at, MouseButton::Left, cx);
                        w.pointer_up(at, MouseButton::Left, cx);
                    })?;
                    Ok(Value::Null)
                }
                "type" => self.with(|w, cx| w.input(arg, cx)).map(|_| Value::Null),
                "key" => self.with(|w, cx| {
                    let keys: Vec<&str> = arg.split_whitespace().collect();
                    let os = cx.platform_info().os;
                    if let Some(k) = keys
                        .iter()
                        .find(|k| Keystroke::parse_with_os(k, os).is_err())
                    {
                        return Err(format!("not a key: {k}"));
                    }
                    keys.iter().for_each(|k| w.press(k, cx));
                    Ok(Value::Null)
                })?,
                "scroll" => {
                    let (id, dy) = arg.rsplit_once(' ').ok_or("scroll <id> <dy>")?;
                    let dy: f32 = dy.parse().map_err(|_| "dy is a number")?;
                    let at = self.with(|w, _| find(w, id))??.bounds().center();
                    self.with(|w, cx| {
                        w.pointer_move(at, None, cx);
                        let delta = ScrollDelta::Pixels(point(px(0.), px(-dy)));
                        let e = ScrollEvent {
                            position: at,
                            delta,
                            ..Default::default()
                        };
                        w.dispatch_event(e.to_platform_input(), cx);
                    })?;
                    Ok(Value::Null)
                }
                "wait" => {
                    let start = self.cx.background_executor().now();
                    while !self.with(|w, _| shown(w, arg))? {
                        if self.cx.background_executor().now() - start > Duration::from_secs(30) {
                            return Err(format!("{arg} did not show in 30 s"));
                        }
                        common::tick(&mut self.cx);
                    }
                    Ok(Value::Null)
                }
                "resize" => {
                    let n: Vec<f32> = arg
                        .split_whitespace()
                        .filter_map(|n| n.parse().ok())
                        .collect();
                    let [width, height] = n[..] else {
                        return Err("resize <w> <h>".into());
                    };
                    self.with(|w, cx| {
                        w.resize(size(px(width), px(height)));
                        w.bounds_changed(cx);
                    })?;
                    Ok(Value::Null)
                }
                "mode" => {
                    let mode = match arg {
                        "light" => Mode::Light,
                        "dark" => Mode::Dark,
                        _ => return Err("mode light|dark".into()),
                    };
                    self.with(|w, cx| theme::set_mode(mode, w, cx))?;
                    Ok(Value::Null)
                }
                "quit" => Ok(Value::Null),
                _ => Err(format!("unknown command: {cmd}")),
            }
        }
    }

    /// `name:3` for a numbered id.
    fn name(id: &ElementId) -> String {
        match id {
            ElementId::NamedInteger(n, i) => format!("{n}:{i}"),
            id => id.to_string(),
        }
    }

    fn parse(s: &str) -> ElementId {
        let numbered = s
            .rsplit_once(':')
            .and_then(|(n, i)| Some((n, i.parse().ok()?)));
        numbered.map_or_else(
            || ElementId::Name(s.to_string().into()),
            |(n, i)| ElementId::NamedInteger(n.to_string().into(), i),
        )
    }

    fn node(e: &ElementSnapshot) -> Value {
        let b = e.bounds();
        let bounds = [b.origin.x, b.origin.y, b.size.width, b.size.height].map(f32::from);
        json!({
            "id": e.path().last().map(name),
            "role": e.role().map(|r| format!("{r:?}")),
            "label": e.label(),
            "value": e.value(),
            "bounds": bounds,
        })
    }

    /// The one visible element `spec` names: an id, or `scope/…/id` with ids along its path.
    fn find(w: &Window, spec: &str) -> Result<ElementSnapshot, String> {
        let ids: Vec<ElementId> = spec.split('/').map(parse).collect();
        let (id, scope) = ids.split_last().ok_or("an element id")?;
        let mut hits: Vec<_> = (w.elements().into_iter())
            .filter(|e| e.visible() && e.path().last() == Some(id))
            .filter(|e| {
                let mut path = e.path().iter();
                scope.iter().all(|s| path.any(|p| p == s))
            })
            .collect();
        match hits.len() {
            1 => Ok(hits.remove(0)),
            0 => Err(format!("no visible {spec}")),
            n => {
                let named = |e: &ElementSnapshot| {
                    let ids = e.path().iter().filter(|id| {
                        matches!(id, ElementId::Name(_) | ElementId::NamedInteger(..))
                    });
                    ids.map(name).collect::<Vec<_>>().join("/")
                };
                let paths: Vec<_> = hits.iter().map(named).collect();
                Err(format!(
                    "{n} visible {spec}, name a scope: {}",
                    paths.join(", ")
                ))
            }
        }
    }

    /// Whether `arg` names a visible element, or a visible label or value holds it.
    fn shown(w: &Window, arg: &str) -> bool {
        find(w, arg).is_ok()
            || w.elements().iter().any(|e| {
                e.visible()
                    && [e.label(), e.value()]
                        .iter()
                        .flatten()
                        .any(|t| t.contains(arg))
            })
    }

    fn send(sock: &Path, cmd: &str) -> Value {
        let mut stream = UnixStream::connect(sock).unwrap();
        writeln!(stream, "{cmd}").unwrap();
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).unwrap();
        serde_json::from_str(&reply).unwrap()
    }

    fn selftest() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("sock");
        let listener = listen(dir.path());
        let cmds = [
            "tree",
            "click top.chat",
            "type pong",
            "key enter",
            "wait said pong",
            "shot chat",
            "click nothing.here",
            "quit",
        ];
        let client = thread::spawn(move || cmds.map(|c| send(&sock, c)));
        serve(listener, dir.path(), 1);
        let [tree, click, typed, key, wait, shot, missing, quit] = client.join().unwrap();
        let ids = tree["ok"].as_array().expect("tree lists elements");
        assert!(ids.iter().any(|e| e["id"] == "top.chat"), "{tree}");
        for ok in [click, typed, key, wait, quit] {
            assert_eq!(ok, json!({ "ok": null }));
        }
        let png = shot["ok"].as_str().expect("shot returns its path");
        assert!(fs::metadata(png).unwrap().len() > 0);
        assert_eq!(missing, json!({ "error": "no visible nothing.here" }));
    }
}
