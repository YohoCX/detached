use detached_core::types::{MergeMethod, MergeRequest, MrAction};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder, px,
};

use super::mr::is_gitlab;
use super::mr_page::{MrPage, button};
use crate::ui::{self, radius, theme};
use kernel::forge::active;

/// The merge status and controls of an open MR; its choices last while its Overview shows.
#[derive(Default)]
pub struct MergeWidget {
    picked: Option<MergeMethod>,
    delete: bool,
    auto: bool,
}

fn running(mr: &MergeRequest) -> bool {
    mr.pipeline.as_ref().is_some_and(active)
}

fn label(m: &MergeMethod) -> &'static str {
    match m {
        MergeMethod::Merge => "Merge commit",
        MergeMethod::Squash => "Squash",
        MergeMethod::Rebase => "Rebase",
    }
}

impl MergeWidget {
    /// The picked method while the MR allows it, else its first.
    fn method(&self, mr: &MergeRequest) -> MergeMethod {
        let picked = self.picked.clone().filter(|m| mr.merge_methods.contains(m));
        picked
            .or_else(|| mr.merge_methods.first().cloned())
            .unwrap_or(MergeMethod::Merge)
    }

    fn auto(&self, mr: &MergeRequest) -> bool {
        self.auto && running(mr)
    }

    // GitHub deletes the branch itself after a merge, which it cannot do after an auto-merge.
    fn can_delete(&self, mr: &MergeRequest) -> bool {
        mr.can_delete_source_branch && (is_gitlab(mr) || !self.auto(mr))
    }

    fn action(&self, mr: &MergeRequest) -> MrAction {
        MrAction::Merge {
            method: self.method(mr),
            delete_source_branch: self.can_delete(mr) && self.delete,
            auto_merge: self.auto(mr),
            head_sha: mr.head_sha.clone(),
        }
    }

    pub fn render(&self, mr: &MergeRequest, busy: bool, cx: &Context<MrPage>) -> impl IntoElement {
        let t = theme(cx);
        let (dot, status) = match (mr.auto_merge, &mr.blocking) {
            (true, _) => (t.accent, "Set to merge when the pipeline succeeds".into()),
            (_, Some(b)) => (t.warning, format!("Cannot merge: {b}")),
            _ => (t.success, "Ready to merge".to_string()),
        };
        let auto = self.auto(mr);
        let methods = mr.merge_methods.clone();
        let options: Vec<_> = methods
            .iter()
            .enumerate()
            .map(|(i, m)| (i, label(m)))
            .collect();
        let at = methods.iter().position(|m| *m == self.method(mr));
        let at = at.unwrap_or(0);
        let page = cx.weak_entity();
        let pick = move |i: usize, _: &mut _, cx: &mut _| {
            page.update(cx, |p: &mut MrPage, cx| {
                p.merge.picked = methods.get(i).cloned();
                cx.notify();
            })
            .ok();
        };
        let check = |id: &'static str, text: &'static str, on: bool, set: fn(&mut Self, bool)| {
            Checkbox::new(id)
                .label(text)
                .checked(on)
                .on_click(cx.listener(move |p, on: &bool, _, cx| {
                    set(&mut p.merge, *on);
                    cx.notify();
                }))
        };
        let action = self.action(mr);
        let controls = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(12.))
            .when(options.len() > 1, |d| {
                d.child(ui::segmented("merge.method", &options, at, pick, cx))
            })
            .when(self.can_delete(mr), |d| {
                d.child(check(
                    "merge.delete",
                    "Delete source branch",
                    self.delete,
                    |m, on| m.delete = on,
                ))
            })
            .when(running(mr), |d| {
                d.child(check(
                    "merge.auto",
                    "When the pipeline succeeds",
                    self.auto,
                    |m, on| m.auto = on,
                ))
            })
            .child(div().flex_1())
            .child(
                button(
                    "merge.submit",
                    if auto { "Set auto-merge" } else { "Merge" },
                    !busy && (mr.blocking.is_none() || auto),
                    cx.listener(move |p, _, _, cx| p.act(action.clone(), cx).detach()),
                    cx,
                )
                .bg(t.primary)
                .text_color(t.primary_fg),
            );
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .px(px(14.))
            .py(px(12.))
            .border_1()
            .border_color(t.line)
            .rounded(radius::FIELD)
            .bg(t.panel)
            .child(
                div()
                    .id("merge.status")
                    .test_support()
                    .aria_label(status.clone())
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .font_weight(FontWeight::BOLD)
                    .child(div().size(px(8.)).rounded_full().bg(dot))
                    .child(status),
            )
            .when(!mr.auto_merge, |d| d.child(controls))
    }
}
