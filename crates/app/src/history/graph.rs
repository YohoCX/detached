//! A log row's slice of the graph, painted on a canvas.

use detached_core::types::{Edge, LogRow};
use gpui_kit::{Bounds, IntoElement, PathBuilder, Styled, canvas, fill, point, px, size};

use crate::ui::theme::lane;

pub const LANE: f32 = 14.;
pub const ROW: f32 = 28.;

/// The row's edges and its dot, filling the space it is given.
pub fn paint(row: &LogRow) -> impl IntoElement {
    let (top, bottom, column, color) = (row.top.clone(), row.bottom.clone(), row.column, row.color);
    canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            let at = |c: u32, y: f32| b.origin + point(px(c as f32 * LANE + LANE / 2.), px(y));
            let halves = [(&top, 0., ROW / 2.), (&bottom, ROW / 2., ROW)];
            // Butt caps: lyon's LineCap is not exported, and each end is clipped or under a dot.
            for (edges, y1, y2) in halves {
                for &Edge { from, to, color } in edges {
                    let mut p = PathBuilder::stroke(px(2.));
                    p.move_to(at(from, y1));
                    let mid = (y1 + y2) / 2.;
                    if from == to {
                        p.line_to(at(to, y2));
                    } else {
                        p.cubic_bezier_to(at(to, y2), at(from, mid), at(to, mid));
                    }
                    if let Ok(path) = p.build() {
                        window.paint_path(path, lane(color));
                    }
                }
            }
            let dot = Bounds::centered_at(at(column, ROW / 2.), size(px(8.), px(8.)));
            window.paint_quad(fill(dot, lane(color)).corner_radii(px(4.)));
        },
    )
    .size_full()
}
