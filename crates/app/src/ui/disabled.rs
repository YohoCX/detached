//! A disabled flag on an element's accessibility node, which GPUI has no setter for.

use gpui_kit::{
    A11ySubtreeBuilder, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId,
    InteractiveElement, Interactivity, IntoElement, LayoutId, Pixels, Role, Window, accesskit,
};

/// `e`, announced as disabled while `off`; it still takes clicks.
pub fn disabled<E: Element>(e: E, off: bool) -> Disabled<E> {
    Disabled(e, off)
}

pub struct Disabled<E>(E, bool);

impl<E: Element> IntoElement for Disabled<E> {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl<E: InteractiveElement> InteractiveElement for Disabled<E> {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.0.interactivity()
    }
}

impl<E: Element> Element for Disabled<E> {
    type RequestLayoutState = E::RequestLayoutState;
    type PrepaintState = E::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        self.0.id()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.0.source_location()
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.0.request_layout(id, inspector, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.0.prepaint(id, inspector, bounds, layout, window, cx)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.0
            .paint(id, inspector, bounds, layout, prepaint, window, cx)
    }

    fn a11y_role(&self) -> Option<Role> {
        self.0.a11y_role()
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        self.0.write_a11y_info(node);
        if self.1 {
            node.set_disabled();
        }
    }

    fn a11y_synthetic_children(
        &mut self,
        prepaint: &mut Self::PrepaintState,
        builder: &mut A11ySubtreeBuilder,
    ) {
        self.0.a11y_synthetic_children(prepaint, builder);
    }
}
