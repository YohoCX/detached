use gpui_kit::{Hsla, IntoElement, PathBuilder, Styled, canvas, point, px};

pub struct Edge {
    pub from: (f32, f32),
    pub to: (f32, f32),
    pub color: Hsla,
}

/// Coordinates are fractions of the graph bounds; nodes remain regular elements above this layer.
pub fn edges(edges: Vec<Edge>) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            let at = |(x, y): (f32, f32)| b.origin + point(b.size.width * x, b.size.height * y);
            for e in &edges {
                let mut p = PathBuilder::stroke(px(2.));
                p.move_to(at(e.from));
                let mid = (e.from.0 + e.to.0) / 2.;
                p.cubic_bezier_to(at(e.to), at((mid, e.from.1)), at((mid, e.to.1)));
                if let Ok(path) = p.build() {
                    window.paint_path(path, e.color);
                }
            }
        },
    )
    .absolute()
    .inset_0()
}
