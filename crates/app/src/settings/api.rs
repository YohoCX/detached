//! What features call to add a Settings page and to open Settings at one.

use gpui_kit::{AnyView, App, Window};

use super::{GlobalOpener, Open, Page, Pages};

/// Where a page is listed, in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    App,
    Modules,
    Repo,
}

impl Group {
    pub(super) fn title(self) -> &'static str {
        match self {
            Group::App => "App",
            Group::Modules => "Modules",
            Group::Repo => "Current repository",
        }
    }
}

pub type View = fn(&mut Window, &mut App) -> AnyView;

/// Adds a page, made each time Settings opens; a group lists its pages in the order they came.
pub fn register_page(
    cx: &mut App,
    id: &'static str,
    title: &'static str,
    group: Group,
    view: View,
) {
    let page = Page {
        id,
        title,
        group,
        view,
    };
    cx.default_global::<Pages>().0.push(page);
}

/// Opens Settings at page `id`, or shows that page when it is open.
pub fn open(cx: &mut App, id: &'static str) {
    if let Some(o) = cx.try_global::<GlobalOpener>() {
        o.0.clone().update(cx, |_, cx| cx.emit(Open(id)));
    }
}
