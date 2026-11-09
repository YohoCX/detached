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
    Folder,
    Repo,
    Tree,
    Changes,
    Mr,
    Chevron,
    Down,
    Check,
    Up,
    Next,
    Zed,
    Search,
    Edit,
    Log,
    Pipeline,
    Gear,
    Terminal,
    Chat,
    PanelLeft,
    PanelBottom,
    PanelRight,
    Copy,
    Archive,
    Trash,
    Question,
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
            Icon::Folder => svg16!("M2 4.5h4l1.5 1.5H14v6.5H2z"),
            Icon::Repo => svg16!("M3.5 2.5h9v11h-9zM6 2.5v11"),
            Icon::Tree => svg16!("M3 3h4M5 3v9h3M5 7.5h3M10 7.5h3M10 12h3"),
            Icon::Changes => svg16!("M3 3h10v10H3zM6 8h4M8 6v4"),
            Icon::Mr => svg16!(
                "M4.5 4.5v7M4.5 3a1.5 1.5 0 1 0 0 .01M4.5 13a1.5 1.5 0 1 0 0 .01M11.5 13a1.5 1.5 0 1 0 0 .01M11.5 11.5V6.5a2 2 0 0 0-2-2H7M8.5 3 7 4.5 8.5 6"
            ),
            Icon::Chevron => svg16!("M6 4l4 4-4 4"),
            Icon::Down => svg16!("M4 6l4 4 4-4"),
            Icon::Check => svg16!("M3.5 8.5l3 3 6-7"),
            Icon::Up => svg16!("M8 13V3M4 7l4-4 4 4"),
            Icon::Next => svg16!("M8 3v10M4 9l4 4 4-4"),
            Icon::Zed => svg16!("M4 4h8L4 12h8"),
            Icon::Search => svg16!("M7 2.5a4.5 4.5 0 1 0 0 9 4.5 4.5 0 0 0 0-9M13.5 13.5l-3.3-3.3"),
            Icon::Edit => svg16!("M10.5 3l2.5 2.5L6 12.5H3.5V10zM9 4.5 11.5 7"),
            Icon::Log => svg16!(
                "M4 2.5v1M4 6.5v3M4 12.5v1M2.5 5a1.5 1.5 0 1 0 3 0 1.5 1.5 0 1 0-3 0M2.5 11a1.5 1.5 0 1 0 3 0 1.5 1.5 0 1 0-3 0M8 5h5.5M8 11h5.5"
            ),
            Icon::Pipeline => svg16!(
                "M1.5 8a1.5 1.5 0 1 0 3 0 1.5 1.5 0 1 0-3 0M6.5 8a1.5 1.5 0 1 0 3 0 1.5 1.5 0 1 0-3 0M11.5 8a1.5 1.5 0 1 0 3 0 1.5 1.5 0 1 0-3 0M4.5 8h2M9.5 8h2"
            ),
            Icon::Gear => svg16!(
                "M8 5.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5M8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M3.4 12.6l1.4-1.4M11.2 4.8l1.4-1.4"
            ),
            Icon::Terminal => svg16!("M2.5 3h11v10h-11zM5 6.5l2 1.5-2 1.5M8.5 10H11"),
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
