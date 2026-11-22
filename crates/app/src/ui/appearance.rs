//! The Appearance page of Settings: the theme, light or dark, and the folder of theme files.

use detached_core::api;
use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    AnyView, App, AppContext, BoxShadow, Context, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, Role, SharedString, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder, px, relative,
};
use kernel::activity::Activity;
use kernel::blocking;

use super::page::{button, card, item, labeled, layout};
use super::theme::{Mode, Theme};
use super::{Icon, radius, segmented, text, theme};

pub fn page(_: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| {
        let dir = kernel::core(cx).config_dir.join("themes");
        let loaded = blocking::run(cx, move || theme::previews(&dir));
        cx.spawn(async move |this, cx| {
            let previews = loaded.await;
            this.update(cx, |a: &mut Appearance, cx| {
                a.previews = previews;
                cx.notify();
            })
        })
        .detach();
        Appearance { previews: vec![] }
    })
    .into()
}

struct Appearance {
    previews: Vec<(String, Theme, Theme)>,
}

impl Appearance {
    /// A radio card for theme `name`, its preview drawn in that theme's own tokens.
    fn swatch(&self, name: String, on: bool, cx: &App) -> impl IntoElement + use<> {
        let t = theme(cx);
        let own = self.previews.iter().find(|p| p.0 == name);
        let own = own.map(|p| if theme::is_dark(cx) { &p.2 } else { &p.1 });
        let bar = |w: f32, h: f32| div().w(relative(w)).h(px(h)).rounded_full();
        let preview = own.map(|o| {
            div()
                .flex()
                .h(px(72.))
                .pt(px(10.))
                .pr(px(10.))
                .pl(px(18.))
                .rounded(radius::ICON_BUTTON)
                .bg(o.window_bg)
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .p(px(10.))
                        .rounded_t(radius::KBD)
                        .bg(o.panel_bg)
                        .child(bar(0.4, 6.).bg(o.accent))
                        .child(bar(0.8, 4.).bg(o.muted.opacity(0.6)))
                        .child(bar(0.6, 4.).bg(o.muted.opacity(0.6))),
                )
        });
        let ring = BoxShadow::new(px(0.), px(0.), t.selected_ring).spread_radius(px(1.));
        let chosen = name.clone();
        div()
            .id(SharedString::from(format!("settings.theme.{name}")))
            .test_support()
            .role(Role::RadioButton)
            .aria_label(name.clone())
            .aria_toggled(on.into())
            .flex()
            .flex_col()
            .gap(px(8.))
            .min_w_0()
            .pt(px(8.))
            .px(px(8.))
            .pb(px(12.))
            .rounded(radius::FIELD)
            .border_1()
            .border_color(if on { t.selected_ring } else { t.line })
            .bg(t.header_bg)
            .cursor_pointer()
            .when(on, |c| c.shadow(vec![ring]))
            .when(!on, |c| c.hover(|s| s.bg(t.hover)))
            .on_click(move |_, _, cx| theme::choose(&chosen, cx))
            .child(preview.unwrap_or_else(|| div().h(px(72.))))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(4.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(name)
                    .when(on, |d| {
                        d.text_color(t.fg)
                            .child(div().text_color(t.accent).child(Icon::Check))
                    }),
            )
    }
}

impl Render for Appearance {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dir = kernel::core(cx).config_dir.join("themes");
        let error = theme::error(cx).map(|e| {
            labeled("settings.themeError", e)
                .text_color(theme(cx).warning)
                .text_size(text::SMALL)
        });
        let (names, current) = theme::names(cx);
        let swatches = names.into_iter().map(|n| {
            let on = n == current;
            self.swatch(n, on, cx)
        });
        let themes = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .px(px(2.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Theme"),
            )
            .child(
                div()
                    .id("settings.theme")
                    .test_support()
                    .role(Role::RadioGroup)
                    .aria_label("Theme")
                    .grid()
                    .grid_cols(3)
                    .gap(px(12.))
                    .children(swatches),
            );
        let modes = [
            (Mode::System, "System"),
            (Mode::Light, "Light"),
            (Mode::Dark, "Dark"),
        ];
        let mode = item("Mode", Some("System follows the OS".into()), cx).child(segmented(
            "settings.mode",
            &modes,
            theme::mode(cx),
            theme::set_mode,
            cx,
        ));
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
        let rows = [mode, folder]
            .into_iter()
            .chain(error.map(|e| item(e, None, cx)));
        layout(
            "settings.appearance",
            "Appearance",
            "How Detached looks.",
            None,
            cx,
        )
        .child(themes)
        .child(card(None, None, rows, cx))
    }
}
