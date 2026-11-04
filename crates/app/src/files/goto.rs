//! Go to Line's field, over a file or a diff.

use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::{
    AppContext, Context, Entity, EventEmitter, InteractiveElement, IntoElement, ParentElement,
    Render, Styled, Subscription, Window, div, px,
};

use crate::ui::{radius, theme};

#[derive(Clone, Copy)]
pub(super) enum Goto {
    /// Enter on a line and column, from 1.
    To(u32, u32),
    /// Enter on no line, or Escape: back to the text.
    Back,
    /// Focus went elsewhere.
    Closed,
}

pub(super) struct GotoLine {
    input: Entity<InputState>,
    _input: Subscription,
}

impl EventEmitter<Goto> for GotoLine {}

impl GotoLine {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Line[:column]"));
        input.update(cx, |i, cx| i.focus(window, cx));
        let events = cx.subscribe(&input, |_, input, e: &InputEvent, cx| match e {
            InputEvent::PressEnter { .. } => {
                let to = parse(&input.read(cx).value());
                cx.emit(to.map_or(Goto::Back, |(line, col)| Goto::To(line, col)));
            }
            InputEvent::Blur => cx.emit(Goto::Closed),
            _ => {}
        });
        GotoLine {
            input,
            _input: events,
        }
    }
}

impl Render for GotoLine {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        div()
            .absolute()
            .top(px(10.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(220.))
                    .rounded(radius::ITEM)
                    .shadow(t.shadow.clone())
                    .on_action(cx.listener(|_, _: &Escape, _, cx| cx.emit(Goto::Back)))
                    .child(Input::new(&self.input)),
            )
    }
}

/// `line[:column]`, each read as JavaScript's parseInt does; a column below 1 is 1.
fn parse(s: &str) -> Option<(u32, u32)> {
    let int = |p: &str| {
        let p = p.trim_start();
        let digits = p.find(|c: char| !c.is_ascii_digit()).unwrap_or(p.len());
        p[..digits].parse().ok().filter(|&n| n > 0)
    };
    let mut parts = s.split(':');
    let line = parts.next().and_then(int)?;
    Some((line, parts.next().and_then(int).unwrap_or(1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_line_and_an_optional_column() {
        assert_eq!(parse("12:3"), Some((12, 3)));
        assert_eq!(parse("12"), Some((12, 1)));
        assert_eq!(parse(" 7x:0"), Some((7, 1)));
        for bad in ["", "0", "-2", "x", ":4"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }
}
