//! The Appearance page of Settings: the theme, and the folder of theme files.

use detached_core::api;
use gpui_kit::component::IndexPath;
use gpui_kit::component::select::{SelectEvent, SelectState};
use gpui_kit::{
    AnyView, App, AppContext, Context, IntoElement, ParentElement, Render, Styled, Subscription,
    Window, div, px,
};
use kernel::activity::Activity;
use kernel::blocking;

use super::page::{Opt, Pick, button, card, item, labeled, layout, select};
use super::{text, theme};

pub fn page(window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| {
        let (names, current) = theme::names(cx);
        let ix = names.iter().position(|n| *n == current).map(IndexPath::new);
        let opts = names.into_iter().map(|n| Opt(n.clone(), n.into()));
        let pick = cx.new(|cx| SelectState::new(opts.collect(), ix, window, cx));
        let chosen = cx.subscribe(&pick, |_, _, e: &SelectEvent<Vec<Opt<String>>>, cx| {
            if let SelectEvent::Confirm(Some(name)) = e {
                theme::choose(name, cx);
            }
        });
        Appearance {
            theme: pick,
            _chosen: chosen,
        }
    })
    .into()
}

struct Appearance {
    theme: Pick,
    _chosen: Subscription,
}

impl Render for Appearance {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dir = kernel::core(cx).config_dir.join("themes");
        let error = theme::error(cx).map(|e| {
            labeled("settings.themeError", e)
                .text_color(theme(cx).warning)
                .text_size(text::SMALL)
        });
        let pick = item("Theme", Some("Light and dark follow the system".into()), cx).child(
            div()
                .w(px(240.))
                .child(select(&self.theme, "settings.theme", cx)),
        );
        let detail = "Each file is a theme over the one it extends; an edit shows at once";
        let folder = item("Theme files", Some(detail.into()), cx).child(button(
            "settings.themes",
            "Open folder",
            true,
            cx,
            move |_, _, cx| {
                let dir = dir.clone();
                let opened = blocking::run(cx, move || {
                    theme::seed(&dir)?;
                    api::open_folder(&dir.to_string_lossy())
                });
                cx.spawn(async move |_, cx| {
                    if let Err(e) = opened.await {
                        cx.update(|cx| {
                            let activity = Activity::global(cx);
                            activity.update(cx, |a, cx| a.say(e.to_string(), true, cx))
                        });
                    }
                })
                .detach();
            },
        ));
        let rows = [pick, folder]
            .into_iter()
            .chain(error.map(|e| item(e, None, cx)));
        let about = "How Detached looks.";
        layout("settings.appearance", "Appearance", about, None, cx)
            .child(card(None, None, rows, cx))
    }
}
