use detached_core::types::{Pipeline, PipelineStatus};
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
    latest: Option<Interest<Vec<Pipeline>>>,
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
        let subscriptions = vec![Workspace::observe(cx, |this, cx| {
            this.sync(cx);
            cx.notify();
        })];
        let mut this = PipelineBadge {
            repo: None,
            latest: None,
            _run: run,
            _subscriptions: subscriptions,
        };
        this.sync(cx);
        this
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        let repo = Workspace::get(cx)
            .session_current()
            .filter(|r| {
                Repo::get(cx, r)
                    .and_then(Repo::status)
                    .is_some_and(|st| st.upstream.is_some())
            })
            .map(String::from);
        if repo != self.repo {
            self.repo = repo;
            self.latest = self
                .repo
                .as_ref()
                .map(|repo| Interest::shared_view(forge::pipelines(cx, repo, None, false), cx));
        }
    }
}

impl Render for PipelineBadge {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let latest = self.latest.as_ref().map(|l| l.read(cx));
        let Some(p) = latest
            .filter(|l| l.error.is_none())
            .and_then(|l| l.value.as_ref()?.first())
        else {
            return div().hidden().into_any_element();
        };
        if p.status != PipelineStatus::Failed {
            return div().hidden().into_any_element();
        }
        let t = theme(cx);
        let (glyph, status) = (glyph(&p.status), name(&p.status));
        let tip = format!("Pipeline #{}: {status}", p.id);
        ui::pressable("pipeline.badge")
            .test_support()
            .role(Role::Button)
            .aria_label(format!("{glyph}{status}"))
            .gap(px(6.))
            .h(px(28.))
            .px(px(8.))
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
