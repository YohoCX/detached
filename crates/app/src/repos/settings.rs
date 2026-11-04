//! The Settings dialog: accounts, the current repo's accounts and merge request remote, removed
//! repositories, the git binary.

use std::collections::HashMap;
use std::iter::once;

use detached_core::api;
use detached_core::error::Result;
use detached_core::settings::Settings;
use detached_core::state::AppState;
use detached_core::types::{
    Account, AccountInput, EnvInfo, ForgeHosts, HostAccount, Provider, VerifyResult,
};
use gpui_kit::base::{ObservedElement, TestSupportExt};
use gpui_kit::component::IndexPath;
use gpui_kit::component::input::{Enter, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::{
    AnyView, App, AppContext, ClickEvent, Context, DismissEvent, Div, ElementId, Entity,
    EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Render, Role, ScrollHandle, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Subscription, Window, anchored, deferred, div, point,
    prelude::FluentBuilder, px,
};

use super::{Section, State, settings_call};
use crate::blocking;
use crate::store::Store;
use crate::ui::{self, Icon, radius, text, theme, theme::mono};

/// Asks for the dialog at a section, opening it if needed.
pub struct Open(pub Section);

impl EventEmitter<Open> for State {}

/// Holds the dialog while it is open; it takes no room in its slot.
pub fn settings(window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| {
        let state = State::global(cx);
        let _open = cx.subscribe_in(
            &state,
            window,
            |host: &mut Host, _, e: &Open, window, cx| {
                if let Some((dialog, _)) = &host.dialog {
                    return dialog.update(cx, |d, cx| d.show(e.0, cx));
                }
                let dialog = cx.new(|cx| Dialog::new(e.0, window, cx));
                let closed = cx.subscribe(&dialog, |host: &mut Host, _, _: &DismissEvent, cx| {
                    host.dialog = None;
                    cx.notify();
                });
                host.dialog = Some((dialog, closed));
                cx.notify();
            },
        );
        Host {
            dialog: None,
            _open,
        }
    })
    .into()
}

struct Host {
    dialog: Option<(Entity<Dialog>, Subscription)>,
    _open: Subscription,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .children(self.dialog.as_ref().map(|d| d.0.clone()))
    }
}

const PROVIDERS: [Provider; 4] = [
    Provider::Github,
    Provider::GithubEnterprise,
    Provider::Gitlab,
    Provider::GitlabSelfHosted,
];

/// The provider's name and its fixed base URL, empty when it has none.
fn provider(p: &Provider) -> (&'static str, &'static str) {
    match p {
        Provider::Github => ("GitHub", "https://github.com"),
        Provider::GithubEnterprise => ("GitHub Enterprise", ""),
        Provider::Gitlab => ("GitLab.com", "https://gitlab.com"),
        Provider::GitlabSelfHosted => ("GitLab self-hosted", ""),
    }
}

/// The username's placeholder.
fn login(p: &Provider) -> &'static str {
    match p {
        Provider::Github | Provider::GithubEnterprise => "Your login",
        _ => "oauth2",
    }
}

fn name(a: &Account) -> String {
    match a.label.is_empty() {
        true => a.username.clone(),
        false => a.label.clone(),
    }
}

/// A select option: its value and its text.
#[derive(Clone)]
struct Opt<V>(V, SharedString);

impl<V: Clone + PartialEq + 'static> SelectItem for Opt<V> {
    type Value = V;

    fn title(&self) -> SharedString {
        self.1.clone()
    }

    fn value(&self) -> &V {
        &self.0
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let id = SharedString::from(format!("option:{}", self.1));
        div().id(id).test_support().child(self.1.clone())
    }
}

type Pick<V = String> = Entity<SelectState<Vec<Opt<V>>>>;

fn select<V: Clone + PartialEq + 'static>(
    pick: &Pick<V>,
    id: impl Into<ElementId>,
    cx: &App,
) -> Select<Vec<Opt<V>>> {
    // As high as `ui::input`.
    Select::new(pick)
        .id(id)
        .h(px(32.))
        .px(px(9.))
        .bg(theme(cx).input)
        .rounded(radius::ITEM)
}

/// What a reload reads.
#[derive(Default)]
struct Data {
    settings: Settings,
    accounts: Vec<Account>,
    hosts: Vec<HostAccount>,
    forge: Option<ForgeHosts>,
    env: Option<EnvInfo>,
}

fn load(core: &AppState, repo: Option<&str>) -> Result<Data> {
    Ok(Data {
        settings: api::get_settings(core),
        accounts: api::list_accounts(core)?,
        hosts: match repo {
            Some(r) => api::repo_accounts(core, r)?,
            None => vec![],
        },
        forge: repo.map(|r| api::forge_hosts(core, r)).transpose()?,
        env: Some(api::env_info(core)),
    })
}

/// The selects of the repo sections, made anew from each reload.
#[derive(Default)]
struct Picks {
    hosts: Vec<Pick>,
    /// The remote's, then one per forge host.
    forge: Option<(Pick, Vec<Pick>)>,
    _subs: Vec<Subscription>,
}

impl Picks {
    fn all(&self) -> impl Iterator<Item = &Pick> {
        let forge = self
            .forge
            .iter()
            .flat_map(|(r, hosts)| once(r).chain(hosts));
        self.hosts.iter().chain(forge)
    }
}

fn picks(d: &Data, repo: &str, window: &mut Window, cx: &mut Context<Dialog>) -> Picks {
    let mut subs = vec![];
    let opt = |v: &str, title: String| Opt(v.to_string(), title.into());
    let hosts = d.hosts.iter().map(|h| {
        let first = match h.ambiguous {
            true => "Choose an account",
            false => "Automatic",
        };
        let fits = d.accounts.iter().filter(|a| a.base_url.contains(&h.host));
        let opts = once(opt("", first.into()))
            .chain(fits.map(|a| opt(&a.id, name(a))))
            .collect();
        let r = repo.to_string();
        let value = h.account_id.as_deref().unwrap_or_default();
        let set = move |core: &AppState, id| api::set_repo_account(core, r.clone(), id);
        pick(opts, value, set, &mut subs, window, cx)
    });
    let hosts = hosts.collect();
    let forge = d.forge.as_ref().filter(|f| !f.hosts.is_empty()).map(|f| {
        let first = f
            .mr_remote
            .as_ref()
            .map_or("Choose a remote".into(), |r| format!("Automatic ({r})"));
        let remotes = f.hosts.iter().flat_map(|h| &h.remotes);
        let opts = once(opt("", first))
            .chain(remotes.map(|r| opt(r, r.clone())))
            .collect();
        let value = d.settings.mr_remotes.get(repo).map_or("", String::as_str);
        let r = repo.to_string();
        let set = move |core: &AppState, v| api::set_mr_remote(core, r.clone(), v);
        let remote = pick(opts, value, set, &mut subs, window, cx);
        let chosen = d.settings.forge_accounts.get(repo);
        let hosts = f.hosts.iter().map(|h| {
            let auto = d
                .accounts
                .iter()
                .find(|a| h.account_id.as_ref() == Some(&a.id));
            let first = match auto {
                _ if h.ambiguous => "Choose an account".into(),
                Some(a) => format!("Automatic ({})", name(a)),
                None => "No account fits".into(),
            };
            let all = d.accounts.iter().map(|a| opt(&a.id, name(a)));
            let opts = once(opt("", first)).chain(all).collect();
            let value = chosen
                .and_then(|c| c.get(&h.host))
                .map_or("", String::as_str);
            let (r, host) = (repo.to_string(), h.host.clone());
            let set = move |core: &AppState, id| {
                api::set_forge_account(core, r.clone(), host.clone(), id)
            };
            pick(opts, value, set, &mut subs, window, cx)
        });
        (remote, hosts.collect())
    });
    Picks {
        hosts,
        forge,
        _subs: subs,
    }
}

/// A select over `opts` at `value`; picking an option saves it through `set` (None for the empty
/// one) and reloads.
fn pick(
    opts: Vec<Opt<String>>,
    value: &str,
    set: impl Fn(&AppState, Option<String>) -> Result<Settings> + Clone + Send + 'static,
    subs: &mut Vec<Subscription>,
    window: &mut Window,
    cx: &mut Context<Dialog>,
) -> Pick {
    let ix = opts.iter().position(|o| o.0 == value).map(IndexPath::new);
    let pick = cx.new(|cx| SelectState::new(opts, ix, window, cx));
    subs.push(cx.subscribe_in(
        &pick,
        window,
        move |this, _, e: &SelectEvent<Vec<Opt<String>>>, window, cx| {
            if let SelectEvent::Confirm(Some(v)) = e {
                let (set, v) = (set.clone(), Some(v.clone()).filter(|v| !v.is_empty()));
                this.call(
                    false,
                    move |core| set(core, v).map(drop),
                    |_, (), _, _| {},
                    window,
                    cx,
                );
            }
        },
    ));
    pick
}

struct Form {
    id: Option<String>,
    provider: Pick<Provider>,
    base: Entity<InputState>,
    user: Entity<InputState>,
    label: Entity<InputState>,
    token: Entity<InputState>,
    _provider: Subscription,
}

fn field(
    value: String,
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .default_value(value)
    })
}

impl Form {
    /// Edits `a`, or a new GitHub account.
    fn new(a: Option<&Account>, window: &mut Window, cx: &mut Context<Dialog>) -> Form {
        let p = a.map_or(Provider::Github, |a| a.provider.clone());
        let text = |f: fn(&Account) -> &String| a.map(f).cloned().unwrap_or_default();
        let base = a.map_or(provider(&p).1.into(), |a| a.base_url.clone());
        let keep = match a {
            Some(_) => "Leave empty to keep the stored token",
            None => "Personal access token",
        };
        let opts = PROVIDERS.map(|p| Opt(p.clone(), provider(&p).0.into()));
        let ix = PROVIDERS.iter().position(|q| *q == p).map(IndexPath::new);
        let select = cx.new(|cx| SelectState::new(opts.to_vec(), ix, window, cx));
        let _provider = cx.subscribe_in(
            &select,
            window,
            |this: &mut Dialog, _, e: &SelectEvent<Vec<Opt<Provider>>>, window, cx| {
                let (SelectEvent::Confirm(Some(p)), Some(f)) = (e, &this.form) else {
                    return;
                };
                f.base
                    .update(cx, |s, cx| s.set_value(provider(p).1, window, cx));
                f.user
                    .update(cx, |s, cx| s.set_placeholder(login(p), window, cx));
                cx.notify();
            },
        );
        Form {
            id: a.map(|a| a.id.clone()),
            provider: select,
            base: field(base, "https://git.example.com/gitlab", window, cx),
            user: field(text(|a| &a.username), login(&p), window, cx),
            label: field(text(|a| &a.label), "Work GitLab", window, cx),
            token: cx.new(|cx| InputState::new(window, cx).masked(true).placeholder(keep)),
            _provider,
        }
    }
}

struct Dialog {
    repo: Option<String>,
    focus: FocusHandle,
    /// The Accounts section's: focus coming into it reloads, for the selects below.
    accounts: FocusHandle,
    scroll: ScrollHandle,
    /// Shown once the first load is in.
    reveal: Option<Section>,
    data: Data,
    picks: Picks,
    git: Entity<InputState>,
    /// Per account; None while checking.
    checks: HashMap<String, Option<VerifyResult>>,
    removing: Option<String>,
    /// The Accounts section's.
    error: Option<String>,
    form: Option<Form>,
    _focus_in: Subscription,
}

impl EventEmitter<DismissEvent> for Dialog {}

impl Dialog {
    fn new(section: Section, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (focus, accounts) = (cx.focus_handle(), cx.focus_handle());
        window.focus(&focus, cx);
        let git = "git binary (empty: from your shell PATH)";
        let mut this = Dialog {
            repo: Store::global(cx).read(cx).current().map(String::from),
            _focus_in: cx.on_focus_in(&accounts, window, Self::reload),
            focus,
            accounts,
            scroll: ScrollHandle::new(),
            reveal: Some(section),
            data: Data::default(),
            picks: Picks::default(),
            git: field(String::new(), git, window, cx),
            checks: HashMap::new(),
            removing: None,
            error: None,
            form: None,
        };
        this.reload(window, cx);
        this
    }

    fn show(&mut self, section: Section, cx: &mut Context<Self>) {
        match section {
            Section::Accounts => self.scroll.set_offset(point(px(0.), px(0.))),
            // 'Account for', or with no repo 'Removed repositories'.
            Section::Repos => self.scroll.scroll_to_top_of_item(1),
            Section::Git => self.scroll.scroll_to_bottom(),
        }
        cx.notify();
    }

    /// Runs `f`, then reloads and hands `then` what `f` returned. Its error goes to the Accounts
    /// section when `inline`, else to the status bar.
    fn call<T: Send + 'static>(
        &mut self,
        inline: bool,
        f: impl FnOnce(&AppState) -> Result<T> + Send + 'static,
        then: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if inline {
            self.error = None;
        }
        let (core, repo) = (Store::global(cx).read(cx).core().clone(), self.repo.clone());
        let done = blocking::run(cx, move || {
            f(&core).map(|t| (t, load(&core, repo.as_deref())))
        });
        cx.spawn_in(window, async move |this, cx| {
            let done = done.await;
            this.update_in(cx, |this, window, cx| {
                match done {
                    Ok((t, data)) => {
                        match data {
                            Ok(data) => this.apply(data, window, cx),
                            Err(e) => say(e.to_string(), cx),
                        }
                        then(this, t, window, cx);
                    }
                    Err(e) if inline => this.error = Some(e.to_string()),
                    Err(e) => say(e.to_string(), cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.call(false, |_| Ok(()), |_, (), _, _| {}, window, cx);
    }

    fn apply(&mut self, data: Data, window: &mut Window, cx: &mut Context<Self>) {
        let git = data.settings.git_path.clone().unwrap_or_default();
        self.git.update(cx, |s, cx| s.set_value(git, window, cx));
        // A select made anew takes over the focus of the one it replaces, else it leaves the dialog.
        let was = self
            .picks
            .all()
            .position(|p| p.focus_handle(cx).contains_focused(window, cx));
        self.picks = match &self.repo {
            Some(r) => picks(&data, r, window, cx),
            None => Picks::default(),
        };
        if let Some(i) = was {
            let focus = self
                .picks
                .all()
                .nth(i)
                .map_or(self.focus.clone(), |p| p.focus_handle(cx));
            window.focus(&focus, cx);
        }
        self.data = data;
        if let Some(s) = self.reveal.take() {
            self.show(s, cx);
        }
    }

    fn check(&mut self, id: String, cx: &mut Context<Self>) {
        self.checks.insert(id.clone(), None);
        let (core, i) = (Store::global(cx).read(cx).core().clone(), id.clone());
        let done = blocking::run(cx, move || api::verify_account(&core, &i));
        cx.spawn(async move |this, cx| {
            let r = done.await.unwrap_or_else(|e| VerifyResult::Failed {
                status: None,
                error: e.to_string(),
            });
            this.update(cx, |this, cx| {
                this.checks.insert(id, Some(r));
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn edit(&mut self, a: Option<&Account>, window: &mut Window, cx: &mut Context<Self>) {
        self.removing = None;
        self.form = Some(Form::new(a, window, cx));
        cx.notify();
    }

    fn close_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.form = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(f) = &self.form else { return };
        let required = [&f.base, &f.user];
        if let Some(empty) = required.into_iter().find(|i| i.read(cx).value().is_empty()) {
            return empty.update(cx, |s, cx| s.focus(window, cx));
        }
        let value = |i: &Entity<InputState>| i.read(cx).value().to_string();
        let token = Some(value(&f.token).trim().to_string()).filter(|t| !t.is_empty());
        let input = AccountInput {
            id: f.id.clone(),
            provider: f
                .provider
                .read(cx)
                .selected_value()
                .cloned()
                .unwrap_or(Provider::Github),
            base_url: value(&f.base),
            username: value(&f.user),
            label: value(&f.label),
            token: token.clone(),
        };
        self.call(
            true,
            move |core| api::save_account(core, input),
            move |this, saved, window, cx| {
                this.close_form(window, cx);
                this.checks.remove(&saved.id);
                if token.is_some() {
                    this.check(saved.id, cx);
                }
            },
            window,
            cx,
        );
    }

    fn remove(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let i = id.clone();
        self.call(
            true,
            move |core| api::remove_account(core, &i),
            move |this, (), _, _| {
                this.removing = None;
                this.checks.remove(&id);
            },
            window,
            cx,
        );
    }

    fn accounts_view(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let editing = self.form.is_some();
        let rows = self.data.accounts.iter().enumerate().map(|(i, a)| {
            let check = self.checks.get(&a.id);
            let state = match check {
                Some(None) => Some(("Checking…".to_string(), t.muted)),
                Some(Some(VerifyResult::Ok { user })) => {
                    Some((format!("Token ok: {user}"), t.added))
                }
                Some(Some(VerifyResult::Failed { status, error })) => {
                    let status = status.map_or(String::new(), |s| format!(" ({s})"));
                    Some((format!("Check failed{status}: {error}"), t.error))
                }
                None => None,
            };
            let title = match a.label.is_empty() {
                true => a.base_url.clone(),
                false => a.label.clone(),
            };
            let about = format!(
                "{} · {} · {}",
                provider(&a.provider).0,
                a.base_url,
                a.username
            );
            let (id, edit, rm) = (a.id.clone(), a.clone(), a.id.clone());
            let remove = match self.removing.as_ref() == Some(&a.id) {
                true => vec![
                    button(
                        "confirm",
                        "Confirm remove",
                        true,
                        cx,
                        move |this, window, cx| this.remove(rm.clone(), window, cx),
                    )
                    .text_color(t.error),
                    button("cancel", "Cancel", true, cx, |this, _, cx| {
                        this.removing = None;
                        cx.notify();
                    }),
                ],
                false => vec![button("remove", "Remove", true, cx, move |this, _, cx| {
                    this.removing = Some(rm.clone());
                    cx.notify();
                })],
            };
            div()
                .id(("accounts.row", i))
                .test_support()
                .aria_label(title.clone())
                .aria_description(about.clone())
                .flex()
                .items_center()
                .gap(px(12.))
                .py(px(10.))
                .px(px(12.))
                .bg(t.hover)
                .border_1()
                .border_color(t.line)
                .rounded(radius::FIELD)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .flex_1()
                        .min_w_0()
                        .child(div().truncate().font_weight(FontWeight::BOLD).child(title))
                        .child(div().truncate().text_color(t.muted).child(about))
                        .children(state.map(|(s, color)| {
                            div()
                                .id("state")
                                .test_support()
                                .aria_label(s.clone())
                                .truncate()
                                .text_color(color)
                                .child(s)
                        })),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .child(button(
                            "verify",
                            "Check token",
                            check != Some(&None),
                            cx,
                            move |this, _, cx| this.check(id.clone(), cx),
                        ))
                        .child(button(
                            "edit",
                            "Edit",
                            !editing,
                            cx,
                            move |this, window, cx| this.edit(Some(&edit), window, cx),
                        ))
                        .children(remove),
                )
        });
        let empty = "No accounts. Add one to fetch and pull over https with a token.";
        div()
            .id("settings.accounts")
            .test_support()
            .track_focus(&self.accounts)
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(heading("Accounts", cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(text::TITLE)
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .child("Accounts"),
                    )
                    .child(button(
                        "accounts.add",
                        "Add account",
                        !editing,
                        cx,
                        |this, window, cx| this.edit(None, window, cx),
                    )),
            )
            .children(
                self.error
                    .clone()
                    .map(|e| labeled("accounts.error", e).text_color(t.error)),
            )
            .children(rows)
            .when(self.data.accounts.is_empty() && !editing, |d| {
                d.child(labeled("accounts.empty", empty).text_color(t.muted))
            })
            .children(self.form.as_ref().map(|f| form_view(f, cx)))
    }

    fn repo_view(&self, repo: &str, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let name = Store::global(cx).read(cx).name_of(repo).to_string();
        let rows = self.data.hosts.iter().zip(&self.picks.hosts).enumerate();
        let none = "No https remotes; git uses your own credentials (ssh agent, keychain).";
        section("settings.repo", format!("Account for {name}"), cx)
            .children(rows.map(|(i, (h, p))| host(h, select(p, ("settings.account", i), cx), cx)))
            .when(self.data.hosts.is_empty(), |s| s.child(muted(none, cx)))
    }

    fn mrs_view(&self, repo: &str, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let name = Store::global(cx).read(cx).name_of(repo).to_string();
        let s = section("settings.mrs", format!("Merge requests for {name}"), cx);
        let (Some(f), Some((remote, picks))) = (&self.data.forge, &self.picks.forge) else {
            return s.child(muted("No remote is on GitLab or GitHub.", cx));
        };
        let rows = f.hosts.iter().zip(picks).enumerate();
        s.child(
            row().child("Remote").child(div().w(px(240.)).child(select(
                remote,
                "settings.mrRemote",
                cx,
            ))),
        )
        .children(rows.map(|(i, (h, p))| host(h, select(p, ("settings.forgeAccount", i), cx), cx)))
    }

    fn removed_view(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let workspaces = &self.data.settings.workspaces;
        let several = workspaces.len() > 1;
        let excluded = workspaces.iter().flat_map(|w| {
            w.excluded_repos
                .iter()
                .map(move |p| (w.name.clone(), p.clone()))
        });
        let rows: Vec<_> = excluded
            .enumerate()
            .map(|(i, (ws, path))| {
                let (w, p) = (ws.clone(), path.clone());
                row()
                    .id(("settings.excluded", i))
                    .test_support()
                    .aria_label(path.clone())
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_family(mono(cx))
                            .text_size(text::SMALL)
                            .child(path),
                    )
                    .when(several, |r| r.child(div().text_color(t.muted).child(ws)))
                    .child(button(
                        "restore",
                        "Restore",
                        true,
                        cx,
                        move |_, window, cx| {
                            let (w, p) = (w.clone(), p.clone());
                            let restored = settings_call(cx, |core| api::include_repo(core, w, p));
                            cx.spawn_in(window, async move |this, cx| {
                                restored.await;
                                this.update_in(cx, |this, window, cx| this.reload(window, cx))
                            })
                            .detach();
                        },
                    ))
            })
            .collect();
        let none = rows.is_empty();
        section("settings.removed", "Removed repositories".into(), cx)
            .children(rows)
            .when(none, |s| s.child(muted("None.", cx)))
    }

    fn git_view(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let env = self.data.env.as_ref().map(|e| {
            let line = format!("{} · {}", e.git_version, e.git_path);
            labeled("settings.env", line)
                .text_color(t.muted)
                .font_family(mono(cx))
                .text_size(text::SMALL)
        });
        section("settings.git", "Git".into(), cx)
            .children(env)
            .child(
                row()
                    .child(
                        div()
                            .flex_1()
                            .child(ui::input(&self.git, cx).id("settings.gitPath")),
                    )
                    .child(button(
                        "settings.apply",
                        "Apply",
                        true,
                        cx,
                        |this, window, cx| {
                            let path = this.git.read(cx).value().trim().to_string();
                            let path = Some(path).filter(|p| !p.is_empty());
                            let set =
                                move |core: &AppState| api::set_git_path(core, path).map(drop);
                            this.call(false, set, |_, (), _, _| {}, window, cx);
                        },
                    )),
            )
    }
}

fn form_view(f: &Form, cx: &mut Context<Dialog>) -> impl IntoElement + use<> {
    let t = theme(cx);
    let p = f.provider.read(cx).selected_value().cloned();
    let fixed = p.is_some_and(|p| !provider(&p).1.is_empty());
    let named = |label: &'static str, field: Div| {
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .text_color(t.muted)
            .child(label)
            .child(field)
    };
    let input =
        |state: &Entity<InputState>, id: &'static str| div().child(ui::input(state, cx).id(id));
    div()
        .id("accounts.form")
        .test_support()
        .flex()
        .flex_col()
        .gap(px(10.))
        .py(px(10.))
        .px(px(12.))
        .bg(t.hover)
        .border_1()
        .border_color(t.line)
        .rounded(radius::FIELD)
        .capture_action(cx.listener(|this, _: &Enter, window, cx| {
            this.save(window, cx);
            cx.stop_propagation();
        }))
        .child(named(
            "Provider",
            div().child(select(&f.provider, "accounts.provider", cx)),
        ))
        .child(named(
            "Base URL",
            div().child(ui::input(&f.base, cx).disabled(fixed).id("accounts.base")),
        ))
        .child(named("Username", input(&f.user, "accounts.user")))
        .child(named("Label", input(&f.label, "accounts.label")))
        .child(named("Token", input(&f.token, "accounts.token")))
        .child(
            div()
                .flex()
                .gap(px(6.))
                .child(
                    ui::primary_button("accounts.save", "Save", cx)
                        .test_support()
                        .role(Role::Button)
                        .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                )
                .child(button(
                    "accounts.cancel",
                    "Cancel",
                    true,
                    cx,
                    |this, window, cx| this.close_form(window, cx),
                )),
        )
}

/// A `ui::button` that runs `click`, or is dimmed and does nothing.
fn button(
    id: impl Into<ElementId>,
    label: &'static str,
    enabled: bool,
    cx: &Context<Dialog>,
    click: impl Fn(&mut Dialog, &mut Window, &mut Context<Dialog>) + 'static,
) -> ObservedElement<Stateful<Div>> {
    ui::button(id, label, cx)
        .test_support()
        .role(Role::Button)
        .aria_label(label)
        .map(|b| match enabled {
            true => b.on_click(
                cx.listener(move |this, _: &ClickEvent, window, cx| click(this, window, cx)),
            ),
            false => b.opacity(0.5),
        })
}

/// Text that tests read by `id`.
fn labeled(id: &'static str, text: impl Into<SharedString>) -> ObservedElement<Stateful<Div>> {
    let text = text.into();
    div()
        .id(id)
        .test_support()
        .aria_label(text.clone())
        .child(text)
}

fn muted(text: &'static str, cx: &App) -> Div {
    div().text_color(theme(cx).muted).child(text)
}

fn heading(title: &str, cx: &App) -> Div {
    div()
        .text_size(text::LABEL)
        .font_weight(FontWeight::BOLD)
        .text_color(theme(cx).muted)
        .child(title.to_uppercase())
}

fn section(id: &'static str, title: String, cx: &App) -> ObservedElement<Stateful<Div>> {
    div()
        .id(id)
        .test_support()
        .flex()
        .flex_col()
        .child(heading(&title, cx).mb(px(10.)))
}

fn row() -> Div {
    div().flex().items_center().gap(px(10.)).mb(px(6.))
}

/// A host, its remotes and the select for it.
fn host(h: &HostAccount, select: impl IntoElement, cx: &App) -> Div {
    row()
        .child(
            div()
                .min_w_0()
                .font_family(mono(cx))
                .text_size(text::SMALL)
                .child(h.host.clone()),
        )
        .child(
            div()
                .text_color(theme(cx).muted)
                .child(h.remotes.join(", ")),
        )
        .child(div().w(px(240.)).child(select))
}

fn close(cx: &Context<Dialog>) -> impl Fn(&ClickEvent, &mut Window, &mut App) + use<> {
    let this = cx.weak_entity();
    move |_, _, cx| {
        this.update(cx, |_, cx| cx.emit(DismissEvent)).ok();
    }
}

fn say(text: String, cx: &mut App) {
    Store::global(cx).update(cx, |s, cx| s.say(text, true, cx));
}

impl Render for Dialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let vp = window.viewport_size();
        let w = px(720.).min(vp.width * 0.92);
        let head = div()
            .flex()
            .items_center()
            .justify_between()
            .pt(px(12.))
            .pb(px(12.))
            .pl(px(20.))
            .pr(px(14.))
            .border_b_1()
            .border_color(t.line)
            .child(
                div()
                    .text_size(text::HEADING)
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child("Settings"),
            )
            .child(
                ui::titled(ui::icon_button("settings.close", Icon::Close, cx), "Close")
                    .test_support()
                    .on_click(close(cx)),
            );
        let repo = self.repo.clone();
        let mrs = repo.clone().filter(|_| self.data.forge.is_some());
        let body = div()
            .id("settings.body")
            .track_scroll(&self.scroll)
            .overflow_y_scroll()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(22.))
            .px(px(20.))
            .py(px(18.))
            .child(self.accounts_view(cx))
            .children(repo.map(|r| self.repo_view(&r, cx)))
            .children(mrs.map(|r| self.mrs_view(&r, cx)))
            .child(self.removed_view(cx))
            .child(self.git_view(cx));
        let scrim = div()
            .id("settings.scrim")
            .test_support()
            .absolute()
            .size_full()
            .occlude()
            .bg(t.scrim)
            .on_click(close(cx));
        let dialog = ui::glass(div(), cx)
            .id("settings")
            .test_support()
            .track_focus(&self.focus)
            .role(Role::Dialog)
            .aria_label("Settings")
            .absolute()
            .top(vp.height * 0.08)
            .left((vp.width - w) / 2.)
            .w(w)
            .max_h(vp.height * 0.84)
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(radius::DIALOG)
            .occlude()
            .on_key_down(cx.listener(|_, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    cx.emit(DismissEvent);
                }
            }))
            .child(head)
            .child(body);
        let all = div().w(vp.width).h(vp.height).child(scrim).child(dialog);
        deferred(anchored().position(point(px(0.), px(0.))).child(all))
    }
}
