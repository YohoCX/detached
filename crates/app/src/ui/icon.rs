//! Stroke icons, drawn in the surrounding text color.

use std::time::Duration;

use gpui_kit::{
    Animation, AnimationExt, App, Hsla, IntoElement, Pixels, RenderOnce, Styled, Transformation,
    Window, percentage, px, svg,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Branch,
    Fetch,
    Pull,
    Refresh,
    Plus,
    Minus,
    Close,
    Stop,
    Folder,
    Home,
    File,
    Repo,
    Tree,
    Mr,
    Chevron,
    Down,
    Check,
    Up,
    Next,
    External,
    Search,
    EyeOff,
    Edit,
    Gear,
    Chat,
    PanelLeft,
    PanelBottom,
    PanelRight,
    Copy,
    Archive,
    Trash,
    Question,
    More,
    Warning,
    Error,
}

macro_rules! svg16 {
    ($d:literal) => {
        concat!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"><path d=""##,
            $d,
            r#""/></svg>"#
        )
    };
}

impl Icon {
    fn svg(self) -> &'static str {
        match self {
            Icon::Branch => svg16!(
                "M5 3v10M5 13a1.5 1.5 0 1 0 0 .01M5 3a1.5 1.5 0 1 0 0 .01M11 5a1.5 1.5 0 1 0 0 .01M11 6.5c0 3-6 2-6 5"
            ),
            Icon::Fetch => svg16!("M8 2v9M4.5 7.5 8 11l3.5-3.5M3 14h10"),
            Icon::Pull => svg16!("M8 2v8M5 7l3 3 3-3M3 10v3h10v-3"),
            Icon::Refresh => svg16!("M13 8a5 5 0 1 1-1.5-3.5M13 2.5v3h-3"),
            Icon::Plus => svg16!("M8 3v10M3 8h10"),
            Icon::Minus => svg16!("M3 8h10"),
            Icon::Close => svg16!("M4 4l8 8M12 4l-8 8"),
            Icon::Stop => svg16!("M4.5 4.5h7v7h-7z"),
            Icon::Home => svg16!("M2.5 7 8 2.5 13.5 7M4 6v7.5h8V6M7 13.5V10h2v3.5"),
            Icon::Folder => svg16!("M2 4.5h4l1.5 1.5H14v6.5H2z"),
            Icon::Repo => svg16!("M3.5 2.5h9v11h-9zM6 2.5v11"),
            Icon::Tree => svg16!("M3 3h4M5 3v9h3M5 7.5h3M10 7.5h3M10 12h3"),
            Icon::Mr => svg16!(
                "M4.5 4.5v7M4.5 3a1.5 1.5 0 1 0 0 .01M4.5 13a1.5 1.5 0 1 0 0 .01M11.5 13a1.5 1.5 0 1 0 0 .01M11.5 11.5V6.5a2 2 0 0 0-2-2H7M8.5 3 7 4.5 8.5 6"
            ),
            Icon::Chevron => svg16!("M6 4l4 4-4 4"),
            Icon::Down => svg16!("M4 6l4 4 4-4"),
            Icon::Check => svg16!("M3.5 8.5l3 3 6-7"),
            Icon::Up => svg16!("M8 13V3M4 7l4-4 4 4"),
            Icon::Next => svg16!("M8 3v10M4 9l4 4 4-4"),
            Icon::External => svg16!("M9.5 2.5h4v4M13.5 2.5 8 8M11.5 9.5v4h-9v-9h4"),
            Icon::File => svg16!("M3.5 1.5h6l3 3v10h-9zM9.5 1.5v3h3"),
            Icon::Search => svg16!("M7 2.5a4.5 4.5 0 1 0 0 9 4.5 4.5 0 0 0 0-9M13.5 13.5l-3.3-3.3"),
            Icon::EyeOff => svg16!(
                "M2 2l12 12M6 3.8A7 7 0 0 1 8 3.5c4 0 6.5 4.5 6.5 4.5a12 12 0 0 1-2 2.5M10 12.2a7 7 0 0 1-2 .3C4 12.5 1.5 8 1.5 8a12 12 0 0 1 2-2.5M6.5 6.5a2.1 2.1 0 0 0 3 3"
            ),
            Icon::Edit => svg16!("M10.5 3l2.5 2.5L6 12.5H3.5V10zM9 4.5 11.5 7"),
            Icon::Gear => svg16!(
                "M6.8 3.1L6.9 1.2L9.1 1.2L9.2 3.1A5.1 5.1 0 0 1 10.6 3.6L12 2.4L13.6 4L12.4 5.4A5.1 5.1 0 0 1 12.9 6.8L14.8 6.9L14.8 9.1L12.9 9.2A5.1 5.1 0 0 1 12.4 10.6L13.6 12L12 13.6L10.6 12.4A5.1 5.1 0 0 1 9.2 12.9L9.1 14.8L6.9 14.8L6.8 12.9A5.1 5.1 0 0 1 5.4 12.4L4 13.6L2.4 12L3.6 10.6A5.1 5.1 0 0 1 3.1 9.2L1.2 9.1L1.2 6.9L3.1 6.8A5.1 5.1 0 0 1 3.6 5.4L2.4 4L4 2.4L5.4 3.6A5.1 5.1 0 0 1 6.8 3.1ZM8 5.8a2.2 2.2 0 1 0 0 4.4 2.2 2.2 0 1 0 0-4.4"
            ),
            Icon::Chat => svg16!("M2.5 3h11v8H7l-3 2.5V11H2.5zM5 6h6M5 8.5h4"),
            Icon::PanelLeft => svg16!("M2.5 3h11v10h-11zM6.5 3v10"),
            Icon::PanelBottom => svg16!("M2.5 3h11v10h-11zM2.5 9.5h11"),
            Icon::PanelRight => svg16!("M2.5 3h11v10h-11zM9.5 3v10"),
            Icon::Copy => svg16!("M5.5 5.5h8v8h-8zM2.5 10.5v-8h8"),
            Icon::Archive => svg16!("M2.5 3.5h11v3h-11zM3.5 6.5v6h9v-6M6.5 9h3"),
            Icon::Question => svg16!(
                "M8 2.5a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11M6.5 6.5a1.5 1.5 0 1 1 2 1.4c-.4.2-.5.5-.5.9v.4M8 11h.01"
            ),
            Icon::Trash => svg16!("M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.5 9h6l.5-9"),
            Icon::More => svg16!(
                "M2.8 8a.9.9 0 1 0 1.8 0 .9.9 0 1 0-1.8 0M7.1 8a.9.9 0 1 0 1.8 0 .9.9 0 1 0-1.8 0M11.4 8a.9.9 0 1 0 1.8 0 .9.9 0 1 0-1.8 0"
            ),
            Icon::Warning => svg16!("M8 2.5 14 13H2zM8 6.5v3M8 11.5h.01"),
            Icon::Error => {
                svg16!("M8 2.5a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11M8 5.5v3M8 10.5h.01")
            }
        }
    }

    pub fn size(self, size: Pixels) -> Glyph {
        Glyph { icon: self, size }
    }
}

#[derive(IntoElement)]
pub struct Glyph {
    icon: Icon,
    size: Pixels,
}

impl RenderOnce for Glyph {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        svg()
            .data(self.icon.svg().as_bytes())
            .size(self.size)
            .flex_none()
            .text_color(window.text_style().color)
    }
}

impl IntoElement for Icon {
    type Element = <Glyph as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        self.size(px(16.)).into_element()
    }
}

const SPINNER: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10" fill="none" stroke="#000" stroke-width="1.5"><path d="M8 8A4.25 4.25 0 1 1 8 2"/></svg>"##;

/// The 10px ring of a running op, turning once every 0.8 s.
pub fn spinner(color: Hsla) -> impl IntoElement {
    svg()
        .data(SPINNER.as_bytes())
        .size(px(10.))
        .flex_none()
        .text_color(color)
        .with_animation(
            "spinner",
            Animation::new(Duration::from_millis(800)).repeat(),
            |s, t| s.with_transformation(Transformation::rotate(percentage(t))),
        )
}
