use detached_core::types::Pipeline;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    Context, InteractiveElement, IntoElement, ParentElement, Render, Role,
    StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};

use super::{color, glyph, name, pipelines::Session};
use crate::actions::{self, Group, Registration};
use crate::dock;
use crate::ui::{self, radius, theme};
use kernel::forge;
use kernel::repo::Repo;
use kernel::resource::Interest;
use kernel::workspace::Workspace;

/// The current branch's latest pipeline; a click opens Pipelines. It also runs `pipeline.run`.
pub struct PipelineBadge {
    repo: Option<String>,
    /// None without an upstream: then the forge lists every branch's pipelines.
    key: Option<String>,
    latest: Option<Interest<Option<Pipeline>>>,
    _run: Registration,
    _subscriptions: Vec<Subscription>,
}

impl PipelineBadge {
    pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
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
        let subscriptions = vec![Workspace::observe(cx, |this, cx| this.sync(cx))];
        let mut this = PipelineBadge {
            repo: None,
            key: None,
            latest: None,
            _run: run,
            _subscriptions: subscriptions,
        };
        this.sync(cx);
        this
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        let repo = Workspace::get(cx).current().map(String::from);
        let key = repo.as_deref().and_then(|r| {
            let st = Repo::get(cx, r)?
                .status()
                .filter(|st| st.upstream.is_some())?;
            Some(format!("{:?} {:?}", st.head, st.branch))
        });
        if (&repo, &key) != (&self.repo, &self.key) {
            (self.repo, self.key) = (repo, key);
            self.latest = self
                .repo
                .as_ref()
                .zip(self.key.as_ref())
                .map(|(repo, key)| Interest::view(forge::latest_pipeline(cx, repo, key), cx));
        }
    }
}

impl Render for PipelineBadge {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let latest = self.latest.as_ref().map(|l| l.read(cx));
        let Some(p) = latest.and_then(|l| l.value.clone().flatten()) else {
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
