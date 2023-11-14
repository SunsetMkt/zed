use crate::{
    point, px, Action, AnyDrag, AnyElement, AnyTooltip, AnyView, AppContext, BorrowAppContext,
    BorrowWindow, Bounds, ClickEvent, Component, DispatchPhase, Element, ElementId, FocusEvent,
    FocusHandle, KeyContext, KeyDownEvent, KeyUpEvent, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentComponent, Pixels, Point, Render, ScrollWheelEvent,
    SharedString, Size, Style, StyleRefinement, Styled, Task, View, ViewContext, Visibility,
};
use collections::HashMap;
use parking_lot::Mutex;
use refineable::Refineable;
use smallvec::SmallVec;
use std::{
    any::{Any, TypeId},
    fmt::Debug,
    marker::PhantomData,
    mem,
    sync::Arc,
    time::Duration,
};
use taffy::style::Overflow;
use util::ResultExt;

const DRAG_THRESHOLD: f64 = 2.;
const TOOLTIP_DELAY: Duration = Duration::from_millis(500);
const TOOLTIP_OFFSET: Point<Pixels> = Point::new(px(10.0), px(8.0));

pub struct GroupStyle {
    pub group: SharedString,
    pub style: StyleRefinement,
}

pub trait InteractiveComponent<V: 'static>: Sized + Element<V> {
    fn interactivity(&mut self) -> &mut Interactivity<V>;

    fn group(mut self, group: impl Into<SharedString>) -> Self {
        self.interactivity().group = Some(group.into());
        self
    }

    fn id(mut self, id: impl Into<ElementId>) -> Stateful<V, Self> {
        self.interactivity().element_id = Some(id.into());

        Stateful {
            element: self,
            view_type: PhantomData,
        }
    }

    fn track_focus(mut self, focus_handle: &FocusHandle) -> Focusable<V, Self> {
        self.interactivity().focusable = true;
        self.interactivity().tracked_focus_handle = Some(focus_handle.clone());
        Focusable {
            element: self,
            view_type: PhantomData,
        }
    }

    fn key_context<C, E>(mut self, key_context: C) -> Self
    where
        C: TryInto<KeyContext, Error = E>,
        E: Debug,
    {
        if let Some(key_context) = key_context.try_into().log_err() {
            self.interactivity().key_context = key_context;
        }
        self
    }

    fn hover(mut self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self {
        self.interactivity().hover_style = f(StyleRefinement::default());
        self
    }

    fn group_hover(
        mut self,
        group_name: impl Into<SharedString>,
        f: impl FnOnce(StyleRefinement) -> StyleRefinement,
    ) -> Self {
        self.interactivity().group_hover_style = Some(GroupStyle {
            group: group_name.into(),
            style: f(StyleRefinement::default()),
        });
        self
    }

    fn on_mouse_down(
        mut self,
        button: MouseButton,
        handler: impl Fn(&mut V, &MouseDownEvent, &mut ViewContext<V>) + 'static,
    ) -> Self {
        self.interactivity().mouse_down_listeners.push(Box::new(
            move |view, event, bounds, phase, cx| {
                if phase == DispatchPhase::Bubble
                    && event.button == button
                    && bounds.contains_point(&event.position)
                {
                    handler(view, event, cx)
                }
            },
        ));
        self
    }

    fn on_any_mouse_down(
        mut self,
        handler: impl Fn(&mut V, &MouseDownEvent, &mut ViewContext<V>) + 'static,
    ) -> Self {
        self.interactivity().mouse_down_listeners.push(Box::new(
            move |view, event, bounds, phase, cx| {
                if phase == DispatchPhase::Bubble && bounds.contains_point(&event.position) {
                    handler(view, event, cx)
                }
            },
        ));
        self
    }

    fn on_mouse_up(
        mut self,
        button: MouseButton,
        handler: impl Fn(&mut V, &MouseUpEvent, &mut ViewContext<V>) + 'static,
    ) -> Self {
        self.interactivity().mouse_up_listeners.push(Box::new(
            move |view, event, bounds, phase, cx| {
                if phase == DispatchPhase::Bubble
                    && event.button == button
                    && bounds.contains_point(&event.position)
                {
                    handler(view, event, cx)
                }
            },
        ));
        self
    }

    fn on_any_mouse_up(
        mut self,
        handler: impl Fn(&mut V, &MouseUpEvent, &mut ViewContext<V>) + 'static,
    ) -> Self {
        self.interactivity().mouse_up_listeners.push(Box::new(
            move |view, event, bounds, phase, cx| {
                if phase == DispatchPhase::Bubble && bounds.contains_point(&event.position) {
                    handler(view, event, cx)
                }
            },
        ));
        self
    }

    fn on_mouse_down_out(
        mut self,
        handler: impl Fn(&mut V, &MouseDownEvent, &mut ViewContext<V>) + 'static,
    ) -> Self {
        self.interactivity().mouse_down_listeners.push(Box::new(
            move |view, event, bounds, phase, cx| {
                if phase == DispatchPhase::Capture && !bounds.contains_point(&event.position) {
                    handler(view, event, cx)
                }
            },
        ));
        self
    }

    fn on_mouse_up_out(
        mut self,
        button: MouseButton,
        handler: impl Fn(&mut V, &MouseUpEvent, &mut ViewContext<V>) + 'static,
    ) -> Self {
        self.interactivity().mouse_up_listeners.push(Box::new(
            move |view, event, bounds, phase, cx| {
                if phase == DispatchPhase::Capture
                    && event.button == button
                    && !bounds.contains_point(&event.position)
                {
                    handler(view, event, cx);
                }
            },
        ));
        self
    }

    fn on_mouse_move(
        mut self,
        handler: impl Fn(&mut V, &MouseMoveEvent, &mut ViewContext<V>) + 'static,
    ) -> Self {
        self.interactivity().mouse_move_listeners.push(Box::new(
            move |view, event, bounds, phase, cx| {
                if phase == DispatchPhase::Bubble && bounds.contains_point(&event.position) {
                    handler(view, event, cx);
                }
            },
        ));
        self
    }

    fn on_scroll_wheel(
        mut self,
        handler: impl Fn(&mut V, &ScrollWheelEvent, &mut ViewContext<V>) + 'static,
    ) -> Self {
        self.interactivity().scroll_wheel_listeners.push(Box::new(
            move |view, event, bounds, phase, cx| {
                if phase == DispatchPhase::Bubble && bounds.contains_point(&event.position) {
                    handler(view, event, cx);
                }
            },
        ));
        self
    }

    /// Capture the given action, fires during the capture phase
    fn capture_action<A: Action>(
        mut self,
        listener: impl Fn(&mut V, &A, &mut ViewContext<V>) + 'static,
    ) -> Self {
        self.interactivity().action_listeners.push((
            TypeId::of::<A>(),
            Box::new(move |view, action, phase, cx| {
                let action = action.downcast_ref().unwrap();
                if phase == DispatchPhase::Capture {
                    listener(view, action, cx)
                }
            }),
        ));
        self
    }

    /// Add a listener for the given action, fires during the bubble event phase
    fn on_action<A: Action>(
        mut self,
        listener: impl Fn(&mut V, &A, &mut ViewContext<V>) + 'static,
    ) -> Self {
        self.interactivity().action_listeners.push((
            TypeId::of::<A>(),
            Box::new(move |view, action, phase, cx| {
                let action = action.downcast_ref().unwrap();
                if phase == DispatchPhase::Bubble {
                    listener(view, action, cx)
                }
            }),
        ));
        self
    }

    fn on_key_down(
        mut self,
        listener: impl Fn(&mut V, &KeyDownEvent, DispatchPhase, &mut ViewContext<V>) + 'static,
    ) -> Self {
        self.interactivity()
            .key_down_listeners
            .push(Box::new(move |view, event, phase, cx| {
                listener(view, event, phase, cx)
            }));
        self
    }

    fn on_key_up(
        mut self,
        listener: impl Fn(&mut V, &KeyUpEvent, DispatchPhase, &mut ViewContext<V>) + 'static,
    ) -> Self {
        self.interactivity()
            .key_up_listeners
            .push(Box::new(move |view, event, phase, cx| {
                listener(view, event, phase, cx)
            }));
        self
    }

    fn drag_over<S: 'static>(mut self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self {
        self.interactivity()
            .drag_over_styles
            .push((TypeId::of::<S>(), f(StyleRefinement::default())));
        self
    }

    fn group_drag_over<S: 'static>(
        mut self,
        group_name: impl Into<SharedString>,
        f: impl FnOnce(StyleRefinement) -> StyleRefinement,
    ) -> Self {
        self.interactivity().group_drag_over_styles.push((
            TypeId::of::<S>(),
            GroupStyle {
                group: group_name.into(),
                style: f(StyleRefinement::default()),
            },
        ));
        self
    }

    fn on_drop<W: 'static>(
        mut self,
        listener: impl Fn(&mut V, View<W>, &mut ViewContext<V>) + 'static,
    ) -> Self {
        self.interactivity().drop_listeners.push((
            TypeId::of::<W>(),
            Box::new(move |view, dragged_view, cx| {
                listener(view, dragged_view.downcast().unwrap(), cx);
            }),
        ));
        self
    }
}

pub trait StatefulInteractiveComponent<V: 'static, E: Element<V>>: InteractiveComponent<V> {
    fn focusable(mut self) -> Focusable<V, Self> {
        self.interactivity().focusable = true;
        Focusable {
            element: self,
            view_type: PhantomData,
        }
    }

    fn overflow_scroll(mut self) -> Self {
        self.interactivity().base_style.overflow.x = Some(Overflow::Scroll);
        self.interactivity().base_style.overflow.y = Some(Overflow::Scroll);
        self
    }

    fn overflow_x_scroll(mut self) -> Self {
        self.interactivity().base_style.overflow.x = Some(Overflow::Scroll);
        self
    }

    fn overflow_y_scroll(mut self) -> Self {
        self.interactivity().base_style.overflow.y = Some(Overflow::Scroll);
        self
    }

    fn active(mut self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self
    where
        Self: Sized,
    {
        self.interactivity().active_style = f(StyleRefinement::default());
        self
    }

    fn group_active(
        mut self,
        group_name: impl Into<SharedString>,
        f: impl FnOnce(StyleRefinement) -> StyleRefinement,
    ) -> Self
    where
        Self: Sized,
    {
        self.interactivity().group_active_style = Some(GroupStyle {
            group: group_name.into(),
            style: f(StyleRefinement::default()),
        });
        self
    }

    fn on_click(
        mut self,
        listener: impl Fn(&mut V, &ClickEvent, &mut ViewContext<V>) + 'static,
    ) -> Self
    where
        Self: Sized,
    {
        self.interactivity()
            .click_listeners
            .push(Box::new(move |view, event, cx| listener(view, event, cx)));
        self
    }

    fn on_drag<W>(
        mut self,
        listener: impl Fn(&mut V, &mut ViewContext<V>) -> View<W> + 'static,
    ) -> Self
    where
        Self: Sized,
        W: 'static + Render,
    {
        debug_assert!(
            self.interactivity().drag_listener.is_none(),
            "calling on_drag more than once on the same element is not supported"
        );
        self.interactivity().drag_listener =
            Some(Box::new(move |view_state, cursor_offset, cx| AnyDrag {
                view: listener(view_state, cx).into(),
                cursor_offset,
            }));
        self
    }

    fn on_hover(mut self, listener: impl 'static + Fn(&mut V, bool, &mut ViewContext<V>)) -> Self
    where
        Self: Sized,
    {
        debug_assert!(
            self.interactivity().hover_listener.is_none(),
            "calling on_hover more than once on the same element is not supported"
        );
        self.interactivity().hover_listener = Some(Box::new(listener));
        self
    }

    fn tooltip<W>(
        mut self,
        build_tooltip: impl Fn(&mut V, &mut ViewContext<V>) -> View<W> + 'static,
    ) -> Self
    where
        Self: Sized,
        W: 'static + Render,
    {
        debug_assert!(
            self.interactivity().tooltip_builder.is_none(),
            "calling tooltip more than once on the same element is not supported"
        );
        self.interactivity().tooltip_builder = Some(Arc::new(move |view_state, cx| {
            build_tooltip(view_state, cx).into()
        }));

        self
    }
}

pub trait FocusableComponent<V: 'static>: InteractiveComponent<V> {
    fn focus(mut self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self
    where
        Self: Sized,
    {
        self.interactivity().focus_style = f(StyleRefinement::default());
        self
    }

    fn focus_in(mut self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self
    where
        Self: Sized,
    {
        self.interactivity().focus_in_style = f(StyleRefinement::default());
        self
    }

    fn in_focus(mut self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self
    where
        Self: Sized,
    {
        self.interactivity().in_focus_style = f(StyleRefinement::default());
        self
    }

    fn on_focus(
        mut self,
        listener: impl Fn(&mut V, &FocusEvent, &mut ViewContext<V>) + 'static,
    ) -> Self
    where
        Self: Sized,
    {
        self.interactivity().focus_listeners.push(Box::new(
            move |view, focus_handle, event, cx| {
                if event.focused.as_ref() == Some(focus_handle) {
                    listener(view, event, cx)
                }
            },
        ));
        self
    }

    fn on_blur(
        mut self,
        listener: impl Fn(&mut V, &FocusEvent, &mut ViewContext<V>) + 'static,
    ) -> Self
    where
        Self: Sized,
    {
        self.interactivity().focus_listeners.push(Box::new(
            move |view, focus_handle, event, cx| {
                if event.blurred.as_ref() == Some(focus_handle) {
                    listener(view, event, cx)
                }
            },
        ));
        self
    }

    fn on_focus_in(
        mut self,
        listener: impl Fn(&mut V, &FocusEvent, &mut ViewContext<V>) + 'static,
    ) -> Self
    where
        Self: Sized,
    {
        self.interactivity().focus_listeners.push(Box::new(
            move |view, focus_handle, event, cx| {
                let descendant_blurred = event
                    .blurred
                    .as_ref()
                    .map_or(false, |blurred| focus_handle.contains(blurred, cx));
                let descendant_focused = event
                    .focused
                    .as_ref()
                    .map_or(false, |focused| focus_handle.contains(focused, cx));

                if !descendant_blurred && descendant_focused {
                    listener(view, event, cx)
                }
            },
        ));
        self
    }

    fn on_focus_out(
        mut self,
        listener: impl Fn(&mut V, &FocusEvent, &mut ViewContext<V>) + 'static,
    ) -> Self
    where
        Self: Sized,
    {
        self.interactivity().focus_listeners.push(Box::new(
            move |view, focus_handle, event, cx| {
                let descendant_blurred = event
                    .blurred
                    .as_ref()
                    .map_or(false, |blurred| focus_handle.contains(blurred, cx));
                let descendant_focused = event
                    .focused
                    .as_ref()
                    .map_or(false, |focused| focus_handle.contains(focused, cx));
                if descendant_blurred && !descendant_focused {
                    listener(view, event, cx)
                }
            },
        ));
        self
    }
}

pub type FocusListeners<V> = SmallVec<[FocusListener<V>; 2]>;

pub type FocusListener<V> =
    Box<dyn Fn(&mut V, &FocusHandle, &FocusEvent, &mut ViewContext<V>) + 'static>;

pub type MouseDownListener<V> = Box<
    dyn Fn(&mut V, &MouseDownEvent, &Bounds<Pixels>, DispatchPhase, &mut ViewContext<V>) + 'static,
>;
pub type MouseUpListener<V> = Box<
    dyn Fn(&mut V, &MouseUpEvent, &Bounds<Pixels>, DispatchPhase, &mut ViewContext<V>) + 'static,
>;

pub type MouseMoveListener<V> = Box<
    dyn Fn(&mut V, &MouseMoveEvent, &Bounds<Pixels>, DispatchPhase, &mut ViewContext<V>) + 'static,
>;

pub type ScrollWheelListener<V> = Box<
    dyn Fn(&mut V, &ScrollWheelEvent, &Bounds<Pixels>, DispatchPhase, &mut ViewContext<V>)
        + 'static,
>;

pub type ClickListener<V> = Box<dyn Fn(&mut V, &ClickEvent, &mut ViewContext<V>) + 'static>;

pub type DragListener<V> =
    Box<dyn Fn(&mut V, Point<Pixels>, &mut ViewContext<V>) -> AnyDrag + 'static>;

type DropListener<V> = dyn Fn(&mut V, AnyView, &mut ViewContext<V>) + 'static;

pub type HoverListener<V> = Box<dyn Fn(&mut V, bool, &mut ViewContext<V>) + 'static>;

pub type TooltipBuilder<V> = Arc<dyn Fn(&mut V, &mut ViewContext<V>) -> AnyView + 'static>;

pub type KeyDownListener<V> =
    Box<dyn Fn(&mut V, &KeyDownEvent, DispatchPhase, &mut ViewContext<V>) + 'static>;

pub type KeyUpListener<V> =
    Box<dyn Fn(&mut V, &KeyUpEvent, DispatchPhase, &mut ViewContext<V>) + 'static>;

pub type ActionListener<V> =
    Box<dyn Fn(&mut V, &dyn Any, DispatchPhase, &mut ViewContext<V>) + 'static>;

pub fn div<V: 'static>() -> Div<V> {
    Div {
        interactivity: Interactivity::default(),
        children: SmallVec::default(),
    }
}

pub struct Div<V> {
    interactivity: Interactivity<V>,
    children: SmallVec<[AnyElement<V>; 2]>,
}

impl<V> Styled for Div<V> {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.interactivity.base_style
    }
}

impl<V: 'static> InteractiveComponent<V> for Div<V> {
    fn interactivity(&mut self) -> &mut Interactivity<V> {
        &mut self.interactivity
    }
}

impl<V: 'static> ParentComponent<V> for Div<V> {
    fn children_mut(&mut self) -> &mut SmallVec<[AnyElement<V>; 2]> {
        &mut self.children
    }
}

impl<V: 'static> Element<V> for Div<V> {
    type ElementState = NodeState;

    fn element_id(&self) -> Option<ElementId> {
        self.interactivity.element_id.clone()
    }

    fn initialize(
        &mut self,
        view_state: &mut V,
        element_state: Option<Self::ElementState>,
        cx: &mut ViewContext<V>,
    ) -> Self::ElementState {
        let interactive_state = self
            .interactivity
            .initialize(element_state.map(|s| s.interactive_state), cx);
        for child in &mut self.children {
            child.initialize(view_state, cx);
        }

        NodeState {
            interactive_state,
            child_layout_ids: SmallVec::new(),
        }
    }

    fn layout(
        &mut self,
        view_state: &mut V,
        element_state: &mut Self::ElementState,
        cx: &mut ViewContext<V>,
    ) -> crate::LayoutId {
        let mut interactivity = mem::take(&mut self.interactivity);
        let layout_id =
            interactivity.layout(&mut element_state.interactive_state, cx, |style, cx| {
                cx.with_text_style(style.text_style().cloned(), |cx| {
                    element_state.child_layout_ids = self
                        .children
                        .iter_mut()
                        .map(|child| child.layout(view_state, cx))
                        .collect::<SmallVec<_>>();
                    cx.request_layout(&style, element_state.child_layout_ids.iter().copied())
                })
            });
        self.interactivity = interactivity;
        layout_id
    }

    fn paint(
        &mut self,
        bounds: Bounds<Pixels>,
        view_state: &mut V,
        element_state: &mut Self::ElementState,
        cx: &mut ViewContext<V>,
    ) {
        let mut child_min = point(Pixels::MAX, Pixels::MAX);
        let mut child_max = Point::default();
        let content_size = if element_state.child_layout_ids.is_empty() {
            bounds.size
        } else {
            for child_layout_id in &element_state.child_layout_ids {
                let child_bounds = cx.layout_bounds(*child_layout_id);
                child_min = child_min.min(&child_bounds.origin);
                child_max = child_max.max(&child_bounds.lower_right());
            }
            (child_max - child_min).into()
        };

        let mut interactivity = mem::take(&mut self.interactivity);
        interactivity.paint(
            bounds,
            content_size,
            &mut element_state.interactive_state,
            cx,
            |style, scroll_offset, cx| {
                if style.visibility == Visibility::Hidden {
                    return;
                }

                let z_index = style.z_index.unwrap_or(0);

                cx.with_z_index(z_index, |cx| {
                    cx.with_z_index(0, |cx| {
                        style.paint(bounds, cx);
                    });
                    cx.with_z_index(1, |cx| {
                        cx.with_text_style(style.text_style().cloned(), |cx| {
                            cx.with_content_mask(style.overflow_mask(bounds), |cx| {
                                cx.with_element_offset(scroll_offset, |cx| {
                                    for child in &mut self.children {
                                        child.paint(view_state, cx);
                                    }
                                })
                            })
                        })
                    })
                })
            },
        );
        self.interactivity = interactivity;
    }
}

impl<V: 'static> Component<V> for Div<V> {
    fn render(self) -> AnyElement<V> {
        AnyElement::new(self)
    }
}

pub struct NodeState {
    child_layout_ids: SmallVec<[LayoutId; 4]>,
    interactive_state: InteractiveElementState,
}

pub struct Interactivity<V> {
    pub element_id: Option<ElementId>,
    pub key_context: KeyContext,
    pub focusable: bool,
    pub tracked_focus_handle: Option<FocusHandle>,
    pub focus_listeners: FocusListeners<V>,
    pub group: Option<SharedString>,
    pub base_style: StyleRefinement,
    pub focus_style: StyleRefinement,
    pub focus_in_style: StyleRefinement,
    pub in_focus_style: StyleRefinement,
    pub hover_style: StyleRefinement,
    pub group_hover_style: Option<GroupStyle>,
    pub active_style: StyleRefinement,
    pub group_active_style: Option<GroupStyle>,
    pub drag_over_styles: SmallVec<[(TypeId, StyleRefinement); 2]>,
    pub group_drag_over_styles: SmallVec<[(TypeId, GroupStyle); 2]>,
    pub mouse_down_listeners: SmallVec<[MouseDownListener<V>; 2]>,
    pub mouse_up_listeners: SmallVec<[MouseUpListener<V>; 2]>,
    pub mouse_move_listeners: SmallVec<[MouseMoveListener<V>; 2]>,
    pub scroll_wheel_listeners: SmallVec<[ScrollWheelListener<V>; 2]>,
    pub key_down_listeners: SmallVec<[KeyDownListener<V>; 2]>,
    pub key_up_listeners: SmallVec<[KeyUpListener<V>; 2]>,
    pub action_listeners: SmallVec<[(TypeId, ActionListener<V>); 8]>,
    pub drop_listeners: SmallVec<[(TypeId, Box<DropListener<V>>); 2]>,
    pub click_listeners: SmallVec<[ClickListener<V>; 2]>,
    pub drag_listener: Option<DragListener<V>>,
    pub hover_listener: Option<HoverListener<V>>,
    pub tooltip_builder: Option<TooltipBuilder<V>>,
}

impl<V> Interactivity<V>
where
    V: 'static,
{
    pub fn initialize(
        &mut self,
        element_state: Option<InteractiveElementState>,
        cx: &mut ViewContext<V>,
    ) -> InteractiveElementState {
        let mut element_state = element_state.unwrap_or_default();

        // Ensure we store a focus handle in our element state if we're focusable.
        // If there's an explicit focus handle we're tracking, use that. Otherwise
        // create a new handle and store it in the element state, which lives for as
        // as frames contain an element with this id.
        if self.focusable {
            element_state.focus_handle.get_or_insert_with(|| {
                self.tracked_focus_handle
                    .clone()
                    .unwrap_or_else(|| cx.focus_handle())
            });
        }
        element_state
    }

    pub fn layout(
        &mut self,
        element_state: &mut InteractiveElementState,
        cx: &mut ViewContext<V>,
        f: impl FnOnce(Style, &mut ViewContext<V>) -> LayoutId,
    ) -> LayoutId {
        let style = self.compute_style(None, element_state, cx);
        cx.with_key_dispatch(
            self.key_context.clone(),
            self.tracked_focus_handle.clone(),
            |_, cx| f(style, cx),
        )
    }

    pub fn paint(
        &mut self,
        bounds: Bounds<Pixels>,
        content_size: Size<Pixels>,
        element_state: &mut InteractiveElementState,
        cx: &mut ViewContext<V>,
        f: impl FnOnce(Style, Point<Pixels>, &mut ViewContext<V>),
    ) {
        let style = self.compute_style(Some(bounds), element_state, cx);

        if let Some(mouse_cursor) = style.mouse_cursor {
            let hovered = bounds.contains_point(&cx.mouse_position());
            if hovered {
                cx.set_cursor_style(mouse_cursor);
            }
        }

        for listener in self.mouse_down_listeners.drain(..) {
            cx.on_mouse_event(move |state, event: &MouseDownEvent, phase, cx| {
                listener(state, event, &bounds, phase, cx);
            })
        }

        for listener in self.mouse_up_listeners.drain(..) {
            cx.on_mouse_event(move |state, event: &MouseUpEvent, phase, cx| {
                listener(state, event, &bounds, phase, cx);
            })
        }

        for listener in self.mouse_move_listeners.drain(..) {
            cx.on_mouse_event(move |state, event: &MouseMoveEvent, phase, cx| {
                listener(state, event, &bounds, phase, cx);
            })
        }

        for listener in self.scroll_wheel_listeners.drain(..) {
            cx.on_mouse_event(move |state, event: &ScrollWheelEvent, phase, cx| {
                listener(state, event, &bounds, phase, cx);
            })
        }

        let hover_group_bounds = self
            .group_hover_style
            .as_ref()
            .and_then(|group_hover| GroupBounds::get(&group_hover.group, cx));

        if let Some(group_bounds) = hover_group_bounds {
            let hovered = group_bounds.contains_point(&cx.mouse_position());
            cx.on_mouse_event(move |_, event: &MouseMoveEvent, phase, cx| {
                if phase == DispatchPhase::Capture {
                    if group_bounds.contains_point(&event.position) != hovered {
                        cx.notify();
                    }
                }
            });
        }

        if self.hover_style.is_some()
            || (cx.active_drag.is_some() && !self.drag_over_styles.is_empty())
        {
            let hovered = bounds.contains_point(&cx.mouse_position());
            cx.on_mouse_event(move |_, event: &MouseMoveEvent, phase, cx| {
                if phase == DispatchPhase::Capture {
                    if bounds.contains_point(&event.position) != hovered {
                        cx.notify();
                    }
                }
            });
        }

        if cx.active_drag.is_some() {
            let drop_listeners = mem::take(&mut self.drop_listeners);
            cx.on_mouse_event(move |view, event: &MouseUpEvent, phase, cx| {
                if phase == DispatchPhase::Bubble && bounds.contains_point(&event.position) {
                    if let Some(drag_state_type) =
                        cx.active_drag.as_ref().map(|drag| drag.view.entity_type())
                    {
                        for (drop_state_type, listener) in &drop_listeners {
                            if *drop_state_type == drag_state_type {
                                let drag = cx
                                    .active_drag
                                    .take()
                                    .expect("checked for type drag state type above");
                                listener(view, drag.view.clone(), cx);
                                cx.notify();
                                cx.stop_propagation();
                            }
                        }
                    }
                }
            });
        }

        let click_listeners = mem::take(&mut self.click_listeners);
        let drag_listener = mem::take(&mut self.drag_listener);

        if !click_listeners.is_empty() || drag_listener.is_some() {
            let pending_mouse_down = element_state.pending_mouse_down.clone();
            let mouse_down = pending_mouse_down.lock().clone();
            if let Some(mouse_down) = mouse_down {
                if let Some(drag_listener) = drag_listener {
                    let active_state = element_state.clicked_state.clone();

                    cx.on_mouse_event(move |view_state, event: &MouseMoveEvent, phase, cx| {
                        if cx.active_drag.is_some() {
                            if phase == DispatchPhase::Capture {
                                cx.notify();
                            }
                        } else if phase == DispatchPhase::Bubble
                            && bounds.contains_point(&event.position)
                            && (event.position - mouse_down.position).magnitude() > DRAG_THRESHOLD
                        {
                            *active_state.lock() = ElementClickedState::default();
                            let cursor_offset = event.position - bounds.origin;
                            let drag = drag_listener(view_state, cursor_offset, cx);
                            cx.active_drag = Some(drag);
                            cx.notify();
                            cx.stop_propagation();
                        }
                    });
                }

                cx.on_mouse_event(move |view_state, event: &MouseUpEvent, phase, cx| {
                    if phase == DispatchPhase::Bubble && bounds.contains_point(&event.position) {
                        let mouse_click = ClickEvent {
                            down: mouse_down.clone(),
                            up: event.clone(),
                        };
                        for listener in &click_listeners {
                            listener(view_state, &mouse_click, cx);
                        }
                    }
                    *pending_mouse_down.lock() = None;
                });
            } else {
                cx.on_mouse_event(move |_state, event: &MouseDownEvent, phase, _cx| {
                    if phase == DispatchPhase::Bubble && bounds.contains_point(&event.position) {
                        *pending_mouse_down.lock() = Some(event.clone());
                    }
                });
            }
        }

        if let Some(hover_listener) = self.hover_listener.take() {
            let was_hovered = element_state.hover_state.clone();
            let has_mouse_down = element_state.pending_mouse_down.clone();

            cx.on_mouse_event(move |view_state, event: &MouseMoveEvent, phase, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let is_hovered =
                    bounds.contains_point(&event.position) && has_mouse_down.lock().is_none();
                let mut was_hovered = was_hovered.lock();

                if is_hovered != was_hovered.clone() {
                    *was_hovered = is_hovered;
                    drop(was_hovered);

                    hover_listener(view_state, is_hovered, cx);
                }
            });
        }

        if let Some(tooltip_builder) = self.tooltip_builder.take() {
            let active_tooltip = element_state.active_tooltip.clone();
            let pending_mouse_down = element_state.pending_mouse_down.clone();

            cx.on_mouse_event(move |_, event: &MouseMoveEvent, phase, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }

                let is_hovered =
                    bounds.contains_point(&event.position) && pending_mouse_down.lock().is_none();
                if !is_hovered {
                    active_tooltip.lock().take();
                    return;
                }

                if active_tooltip.lock().is_none() {
                    let task = cx.spawn({
                        let active_tooltip = active_tooltip.clone();
                        let tooltip_builder = tooltip_builder.clone();

                        move |view, mut cx| async move {
                            cx.background_executor().timer(TOOLTIP_DELAY).await;
                            view.update(&mut cx, move |view_state, cx| {
                                active_tooltip.lock().replace(ActiveTooltip {
                                    waiting: None,
                                    tooltip: Some(AnyTooltip {
                                        view: tooltip_builder(view_state, cx),
                                        cursor_offset: cx.mouse_position() + TOOLTIP_OFFSET,
                                    }),
                                });
                                cx.notify();
                            })
                            .ok();
                        }
                    });
                    active_tooltip.lock().replace(ActiveTooltip {
                        waiting: Some(task),
                        tooltip: None,
                    });
                }
            });

            if let Some(active_tooltip) = element_state.active_tooltip.lock().as_ref() {
                if active_tooltip.tooltip.is_some() {
                    cx.active_tooltip = active_tooltip.tooltip.clone()
                }
            }
        }

        let active_state = element_state.clicked_state.clone();
        if !active_state.lock().is_clicked() {
            cx.on_mouse_event(move |_, _: &MouseUpEvent, phase, cx| {
                if phase == DispatchPhase::Capture {
                    *active_state.lock() = ElementClickedState::default();
                    cx.notify();
                }
            });
        } else {
            let active_group_bounds = self
                .group_active_style
                .as_ref()
                .and_then(|group_active| GroupBounds::get(&group_active.group, cx));
            cx.on_mouse_event(move |_view, down: &MouseDownEvent, phase, cx| {
                if phase == DispatchPhase::Bubble {
                    let group = active_group_bounds
                        .map_or(false, |bounds| bounds.contains_point(&down.position));
                    let element = bounds.contains_point(&down.position);
                    if group || element {
                        *active_state.lock() = ElementClickedState { group, element };
                        cx.notify();
                    }
                }
            });
        }

        let overflow = style.overflow;
        if overflow.x == Overflow::Scroll || overflow.y == Overflow::Scroll {
            let scroll_offset = element_state
                .scroll_offset
                .get_or_insert_with(Arc::default)
                .clone();
            let line_height = cx.line_height();
            let scroll_max = (content_size - bounds.size).max(&Size::default());

            cx.on_mouse_event(move |_, event: &ScrollWheelEvent, phase, cx| {
                if phase == DispatchPhase::Bubble && bounds.contains_point(&event.position) {
                    let mut scroll_offset = scroll_offset.lock();
                    let old_scroll_offset = *scroll_offset;
                    let delta = event.delta.pixel_delta(line_height);

                    if overflow.x == Overflow::Scroll {
                        scroll_offset.x =
                            (scroll_offset.x + delta.x).clamp(-scroll_max.width, px(0.));
                    }

                    if overflow.y == Overflow::Scroll {
                        scroll_offset.y =
                            (scroll_offset.y + delta.y).clamp(-scroll_max.height, px(0.));
                    }

                    if *scroll_offset != old_scroll_offset {
                        cx.notify();
                        cx.stop_propagation();
                    }
                }
            });
        }

        if let Some(group) = self.group.clone() {
            GroupBounds::push(group, bounds, cx);
        }

        let scroll_offset = element_state
            .scroll_offset
            .as_ref()
            .map(|scroll_offset| *scroll_offset.lock());

        cx.with_key_dispatch(
            self.key_context.clone(),
            element_state.focus_handle.clone(),
            |_, cx| {
                for listener in self.key_down_listeners.drain(..) {
                    cx.on_key_event(move |state, event: &KeyDownEvent, phase, cx| {
                        listener(state, event, phase, cx);
                    })
                }

                for listener in self.key_up_listeners.drain(..) {
                    cx.on_key_event(move |state, event: &KeyUpEvent, phase, cx| {
                        listener(state, event, phase, cx);
                    })
                }

                for (action_type, listener) in self.action_listeners.drain(..) {
                    cx.on_action(action_type, listener)
                }

                if let Some(focus_handle) = element_state.focus_handle.as_ref() {
                    for listener in self.focus_listeners.drain(..) {
                        let focus_handle = focus_handle.clone();
                        cx.on_focus_changed(move |view, event, cx| {
                            listener(view, &focus_handle, event, cx)
                        });
                    }
                }

                f(style, scroll_offset.unwrap_or_default(), cx)
            },
        );

        if let Some(group) = self.group.as_ref() {
            GroupBounds::pop(group, cx);
        }
    }

    pub fn compute_style(
        &self,
        bounds: Option<Bounds<Pixels>>,
        element_state: &mut InteractiveElementState,
        cx: &mut ViewContext<V>,
    ) -> Style {
        let mut style = Style::default();
        style.refine(&self.base_style);

        if let Some(focus_handle) = self.tracked_focus_handle.as_ref() {
            if focus_handle.contains_focused(cx) {
                style.refine(&self.focus_in_style);
            }

            if focus_handle.within_focused(cx) {
                style.refine(&self.in_focus_style);
            }

            if focus_handle.is_focused(cx) {
                style.refine(&self.focus_style);
            }
        }

        if let Some(bounds) = bounds {
            let mouse_position = cx.mouse_position();
            if let Some(group_hover) = self.group_hover_style.as_ref() {
                if let Some(group_bounds) = GroupBounds::get(&group_hover.group, cx) {
                    if group_bounds.contains_point(&mouse_position) {
                        style.refine(&group_hover.style);
                    }
                }
            }
            if bounds.contains_point(&mouse_position) {
                style.refine(&self.hover_style);
            }

            if let Some(drag) = cx.active_drag.take() {
                for (state_type, group_drag_style) in &self.group_drag_over_styles {
                    if let Some(group_bounds) = GroupBounds::get(&group_drag_style.group, cx) {
                        if *state_type == drag.view.entity_type()
                            && group_bounds.contains_point(&mouse_position)
                        {
                            style.refine(&group_drag_style.style);
                        }
                    }
                }

                for (state_type, drag_over_style) in &self.drag_over_styles {
                    if *state_type == drag.view.entity_type()
                        && bounds.contains_point(&mouse_position)
                    {
                        style.refine(drag_over_style);
                    }
                }

                cx.active_drag = Some(drag);
            }
        }

        let clicked_state = element_state.clicked_state.lock();
        if clicked_state.group {
            if let Some(group) = self.group_active_style.as_ref() {
                style.refine(&group.style)
            }
        }

        if clicked_state.element {
            style.refine(&self.active_style)
        }

        style
    }
}

impl<V: 'static> Default for Interactivity<V> {
    fn default() -> Self {
        Self {
            element_id: None,
            key_context: KeyContext::default(),
            focusable: false,
            tracked_focus_handle: None,
            focus_listeners: SmallVec::default(),
            // scroll_offset: Point::default(),
            group: None,
            base_style: StyleRefinement::default(),
            focus_style: StyleRefinement::default(),
            focus_in_style: StyleRefinement::default(),
            in_focus_style: StyleRefinement::default(),
            hover_style: StyleRefinement::default(),
            group_hover_style: None,
            active_style: StyleRefinement::default(),
            group_active_style: None,
            drag_over_styles: SmallVec::new(),
            group_drag_over_styles: SmallVec::new(),
            mouse_down_listeners: SmallVec::new(),
            mouse_up_listeners: SmallVec::new(),
            mouse_move_listeners: SmallVec::new(),
            scroll_wheel_listeners: SmallVec::new(),
            key_down_listeners: SmallVec::new(),
            key_up_listeners: SmallVec::new(),
            action_listeners: SmallVec::new(),
            drop_listeners: SmallVec::new(),
            click_listeners: SmallVec::new(),
            drag_listener: None,
            hover_listener: None,
            tooltip_builder: None,
        }
    }
}

#[derive(Default)]
pub struct InteractiveElementState {
    pub focus_handle: Option<FocusHandle>,
    pub clicked_state: Arc<Mutex<ElementClickedState>>,
    pub hover_state: Arc<Mutex<bool>>,
    pub pending_mouse_down: Arc<Mutex<Option<MouseDownEvent>>>,
    pub scroll_offset: Option<Arc<Mutex<Point<Pixels>>>>,
    pub active_tooltip: Arc<Mutex<Option<ActiveTooltip>>>,
}

pub struct ActiveTooltip {
    #[allow(unused)] // used to drop the task
    waiting: Option<Task<()>>,
    tooltip: Option<AnyTooltip>,
}

/// Whether or not the element or a group that contains it is clicked by the mouse.
#[derive(Copy, Clone, Default, Eq, PartialEq)]
pub struct ElementClickedState {
    pub group: bool,
    pub element: bool,
}

impl ElementClickedState {
    fn is_clicked(&self) -> bool {
        self.group || self.element
    }
}

#[derive(Default)]
pub struct GroupBounds(HashMap<SharedString, SmallVec<[Bounds<Pixels>; 1]>>);

impl GroupBounds {
    pub fn get(name: &SharedString, cx: &mut AppContext) -> Option<Bounds<Pixels>> {
        cx.default_global::<Self>()
            .0
            .get(name)
            .and_then(|bounds_stack| bounds_stack.last())
            .cloned()
    }

    pub fn push(name: SharedString, bounds: Bounds<Pixels>, cx: &mut AppContext) {
        cx.default_global::<Self>()
            .0
            .entry(name)
            .or_default()
            .push(bounds);
    }

    pub fn pop(name: &SharedString, cx: &mut AppContext) {
        cx.default_global::<Self>().0.get_mut(name).unwrap().pop();
    }
}

pub struct Focusable<V, E> {
    element: E,
    view_type: PhantomData<V>,
}

impl<V: 'static, E: InteractiveComponent<V>> FocusableComponent<V> for Focusable<V, E> {}

impl<V, E> InteractiveComponent<V> for Focusable<V, E>
where
    V: 'static,
    E: InteractiveComponent<V>,
{
    fn interactivity(&mut self) -> &mut Interactivity<V> {
        self.element.interactivity()
    }
}

impl<V: 'static, E: StatefulInteractiveComponent<V, E>> StatefulInteractiveComponent<V, E>
    for Focusable<V, E>
{
}

impl<V, E> Styled for Focusable<V, E>
where
    V: 'static,
    E: Styled,
{
    fn style(&mut self) -> &mut StyleRefinement {
        self.element.style()
    }
}

impl<V, E> Element<V> for Focusable<V, E>
where
    V: 'static,
    E: Element<V>,
{
    type ElementState = E::ElementState;

    fn element_id(&self) -> Option<ElementId> {
        self.element.element_id()
    }

    fn initialize(
        &mut self,
        view_state: &mut V,
        element_state: Option<Self::ElementState>,
        cx: &mut ViewContext<V>,
    ) -> Self::ElementState {
        self.element.initialize(view_state, element_state, cx)
    }

    fn layout(
        &mut self,
        view_state: &mut V,
        element_state: &mut Self::ElementState,
        cx: &mut ViewContext<V>,
    ) -> LayoutId {
        self.element.layout(view_state, element_state, cx)
    }

    fn paint(
        &mut self,
        bounds: Bounds<Pixels>,
        view_state: &mut V,
        element_state: &mut Self::ElementState,
        cx: &mut ViewContext<V>,
    ) {
        self.element.paint(bounds, view_state, element_state, cx);
    }
}

impl<V, E> Component<V> for Focusable<V, E>
where
    V: 'static,
    E: 'static + Element<V>,
{
    fn render(self) -> AnyElement<V> {
        AnyElement::new(self)
    }
}

impl<V, E> ParentComponent<V> for Focusable<V, E>
where
    V: 'static,
    E: ParentComponent<V>,
{
    fn children_mut(&mut self) -> &mut SmallVec<[AnyElement<V>; 2]> {
        self.element.children_mut()
    }
}

pub struct Stateful<V, E> {
    element: E,
    view_type: PhantomData<V>,
}

impl<V, E> Styled for Stateful<V, E>
where
    V: 'static,
    E: Styled,
{
    fn style(&mut self) -> &mut StyleRefinement {
        self.element.style()
    }
}

impl<V, E> StatefulInteractiveComponent<V, E> for Stateful<V, E>
where
    V: 'static,
    E: Element<V>,
    Self: InteractiveComponent<V>,
{
}

impl<V, E> InteractiveComponent<V> for Stateful<V, E>
where
    V: 'static,
    E: InteractiveComponent<V>,
{
    fn interactivity(&mut self) -> &mut Interactivity<V> {
        self.element.interactivity()
    }
}

impl<V: 'static, E: FocusableComponent<V>> FocusableComponent<V> for Stateful<V, E> {}

impl<V, E> Element<V> for Stateful<V, E>
where
    V: 'static,
    E: Element<V>,
{
    type ElementState = E::ElementState;

    fn element_id(&self) -> Option<ElementId> {
        self.element.element_id()
    }

    fn initialize(
        &mut self,
        view_state: &mut V,
        element_state: Option<Self::ElementState>,
        cx: &mut ViewContext<V>,
    ) -> Self::ElementState {
        self.element.initialize(view_state, element_state, cx)
    }

    fn layout(
        &mut self,
        view_state: &mut V,
        element_state: &mut Self::ElementState,
        cx: &mut ViewContext<V>,
    ) -> LayoutId {
        self.element.layout(view_state, element_state, cx)
    }

    fn paint(
        &mut self,
        bounds: Bounds<Pixels>,
        view_state: &mut V,
        element_state: &mut Self::ElementState,
        cx: &mut ViewContext<V>,
    ) {
        self.element.paint(bounds, view_state, element_state, cx)
    }
}

impl<V, E> Component<V> for Stateful<V, E>
where
    V: 'static,
    E: 'static + Element<V>,
{
    fn render(self) -> AnyElement<V> {
        AnyElement::new(self)
    }
}

impl<V, E> ParentComponent<V> for Stateful<V, E>
where
    V: 'static,
    E: ParentComponent<V>,
{
    fn children_mut(&mut self) -> &mut SmallVec<[AnyElement<V>; 2]> {
        self.element.children_mut()
    }
}
