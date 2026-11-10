use detached_core::api;
use detached_core::types::Pipeline;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    Context, InteractiveElement, IntoElement, ParentElement, Render, Role,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div, px,
};

use super::{active, color, core, glyph, name, pipelines::Session, poll};
use crate::actions::{self, Group, Registration};
use crate::cache::{self, Cache};
use crate::dock;
use crate::store::Store;
use crate::ui::{self, radius, theme};

/// The current branch's latest pipeline; a click opens Pipelines. It also runs `pipeline.run`.
pub struct PipelineBadge {
    repo: Option<String>,
    /// None without an upstream: then the forge lists every branch's pipelines.
    key: Option<String>,
    poll: Option<Task<()>>,
    _run: Registration,
    _subscriptions: Vec<Subscription>,
}

impl PipelineBadge {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let run = actions::register(
            cx,
            "pipeline.run",
            "Run Pipeline…",
            Group::Vcs,
            |window, cx| {
                cx.default_global::<Session>().run = true;
                dock::open("pipelines", window, cx);
            },
        );
        let subscriptions = vec![
            cx.observe(&Store::global(cx), |this, _, cx| this.sync(cx)),
            cx.observe_global::<Cache<Option<Pipeline>>>(|_, cx| cx.notify()),
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.load(true, cx);
                }
            }),
        ];
        let mut this = PipelineBadge {
            repo: None,
            key: None,
            poll: None,
            _run: run,
            _subscriptions: subscriptions,
        };
        this.sync(cx);
        this
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        let s = Store::global(cx).read(cx);
        let repo = s.current().map(String::from);
        let key = repo.as_deref().and_then(|r| {
            let st = s.status(r).filter(|st| st.upstream.is_some())?;
            Some(format!("badge {r} {:?} {:?}", st.head, st.branch))
        });
        if key != self.key {
            (self.repo, self.key) = (repo, key);
            self.load(false, cx);
        }
    }

    fn load(&mut self, focus: bool, cx: &mut Context<Self>) {
        let (Some(repo), Some(key)) = (self.repo.clone(), self.key.clone()) else {
            return;
        };
        let core = core(cx);
        let latest = move || Ok(api::list_pipelines(&core, &repo, false)?.into_iter().next());
        cache::refresh(key, focus, latest, cx).detach();
    }
}

impl Render for PipelineBadge {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cached = self
            .key
            .as_ref()
            .and_then(|k| cache::cached::<Option<Pipeline>>(k, cx));
        let p = cached.and_then(|c| c.value.clone().flatten());
        let running = p.as_ref().is_some_and(|p| active(&p.status));
        if running != self.poll.is_some() {
            self.poll = running.then(|| poll(5, |this, cx| this.load(false, cx), window, cx));
        }
        let Some(p) = p else {
            return div().hidden().into_any_element();
        };
        let t = theme(cx);
        let (glyph, status) = (glyph(&p.status), name(&p.status));
        let tip = format!("Pipeline #{}: {status}", p.id);
        ui::pressable("pipeline.badge")
            .test_support()
            .role(Role::Button)
            .aria_label(format!("{glyph}{status}"))
            .gap(px(6.))
            .h(px(32.))
            .px(px(10.))
            .rounded(radius::WIDGET)
            .whitespace_nowrap()
            .hover(|s| s.bg(t.hover))
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(|_, window, cx| dock::open("pipelines", window, cx))
            .child(div().text_color(color(&p.status, cx)).child(glyph))
            .child(status)
            .into_any_element()
    }
}
