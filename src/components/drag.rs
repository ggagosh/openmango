//! What every drag in the app shares: a ghost that lifts off in place, a closed hand while
//! dragging, Escape to cancel, and lists that scroll when a drag nears their edge.
//!
//! gpui supplies the drag itself (`on_drag`, `on_drop`, `drag_over`); none of these come with it.
//!
//! A drag preview here is a ghost of what was grabbed, and it stays where it was grabbed. gpui
//! draws a preview at the dragged element's own origin, so that comes free as long as two things
//! hold: the drag source is the thing itself (a row's text, not the full-width column around it),
//! and the ghost starts with whatever the source starts with (a grip, the text). Then the pointer
//! stays on the ghost and the ghost covers what it came from. A stand-in placed beside the
//! pointer was tried first and felt detached at any distance.

use std::cell::Cell;
use std::time::{Duration, Instant};

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;

use crate::theme::{borders, spacing};

/// See-through enough to read what the ghost passes over, solid enough to read the ghost.
pub const GHOST_OPACITY: f32 = 0.85;
/// How close to a list's edge a drag starts scrolling it.
const EDGE_ZONE: f32 = 36.0;
/// Scroll speed with the pointer at, or past, the edge. It ramps up from zero across the zone.
const MAX_SPEED: f32 = 900.0;

/// The surface a drag ghost sits on: a piece of the interface lifted off the page. The shadow
/// carries the lift; the caller adds padding to suit what the ghost starts with.
pub fn ghost(cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .gap(spacing::xs())
        .rounded(borders::radius_sm())
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().popover)
        .text_color(cx.theme().popover_foreground)
        .shadow_md()
        .opacity(GHOST_OPACITY)
        .whitespace_nowrap()
}

/// Show a closed hand for the drag an `on_drag` constructor is starting. Without it the drag
/// keeps whatever cursor its source had, which differs from one source to the next.
pub fn closed_hand_while_dragging(window: &mut Window, cx: &mut App) {
    // The drag exists only once the constructor that called this has returned.
    window.defer(cx, |window, cx| {
        cx.set_active_drag_cursor_style(CursorStyle::ClosedHand, window);
    });
}

/// Escape puts a drag back where it came from. Registered once, by the root view; it runs
/// before any view's own Escape, so cancelling a drag over a panel does not also close it.
pub fn cancel_drag_on_escape(cx: &mut App) -> Subscription {
    cx.intercept_keystrokes(|event, window, cx| {
        if event.keystroke.key == "escape" && cx.stop_active_drag(window) {
            cx.stop_propagation();
        }
    })
}

/// Scrolls a list while a `T` is dragged near its edge, for as long as the pointer stays there.
pub trait DragAutoscroll: InteractiveElement + Sized {
    fn autoscroll_on_drag<T: 'static>(mut self, handle: &ScrollHandle, axis: Axis) -> Self {
        let handle = handle.clone();
        self.interactivity().on_drag_move::<T>(move |event, window, cx| {
            // Only one drag exists at a time, so one loop is enough however often this fires.
            if !AUTOSCROLLING.replace(true) {
                // Count the first step as one frame long, or a loop restarted by every mouse
                // move would only ever take steps of no time at all.
                let a_frame_ago = Instant::now() - Duration::from_millis(16);
                autoscroll_tick(handle.clone(), axis, event.bounds, a_frame_ago, window, cx);
            }
        });
        self
    }
}

impl<E: InteractiveElement + Sized> DragAutoscroll for E {}

thread_local! {
    static AUTOSCROLLING: Cell<bool> = const { Cell::new(false) };
}

/// One frame of scrolling, which schedules the next while the pointer stays in the edge zone.
/// A pointer held still sends no events, so the loop, not the mouse, keeps the list moving.
///
/// ponytail: views that track "which row is the drag over" from mouse moves don't hear about
/// rows that scroll under a still pointer; the next pixel of movement corrects them.
fn autoscroll_tick(
    handle: ScrollHandle,
    axis: Axis,
    bounds: Bounds<Pixels>,
    last_frame: Instant,
    window: &mut Window,
    cx: &mut App,
) {
    let now = Instant::now();
    let speed = if cx.has_active_drag() {
        edge_scroll_speed(window.mouse_position(), bounds, axis)
    } else {
        0.0
    };
    let seconds = now.duration_since(last_frame).as_secs_f32().min(0.05);
    let before = handle.offset();
    let mut offset = before;
    let max = handle.max_offset();
    // A scroll offset runs from zero down to minus the scrollable distance.
    match axis {
        Axis::Vertical => offset.y = (offset.y - px(speed * seconds)).clamp(-max.y, px(0.0)),
        Axis::Horizontal => offset.x = (offset.x - px(speed * seconds)).clamp(-max.x, px(0.0)),
    }
    if speed == 0.0 || offset == before {
        AUTOSCROLLING.set(false);
        return;
    }
    handle.set_offset(offset);
    window.refresh();
    window.on_next_frame(move |window, cx| autoscroll_tick(handle, axis, bounds, now, window, cx));
}

/// Signed scroll speed in pixels per second for a pointer over a list: negative toward the
/// start, positive toward the end, zero away from both edges or off to the side of the list.
fn edge_scroll_speed(pointer: Point<Pixels>, bounds: Bounds<Pixels>, axis: Axis) -> f32 {
    let (position, start, end, across, across_start, across_end) = match axis {
        Axis::Vertical => {
            (pointer.y, bounds.top(), bounds.bottom(), pointer.x, bounds.left(), bounds.right())
        }
        Axis::Horizontal => {
            (pointer.x, bounds.left(), bounds.right(), pointer.y, bounds.top(), bounds.bottom())
        }
    };
    if across < across_start || across > across_end {
        return 0.0;
    }
    edge_speed(f32::from(position), f32::from(start), f32::from(end))
}

fn edge_speed(position: f32, start: f32, end: f32) -> f32 {
    // A short list keeps a middle third that scrolls nothing.
    let zone = EDGE_ZONE.min((end - start) / 3.0);
    if zone <= 0.0 || position < start - EDGE_ZONE || position > end + EDGE_ZONE {
        return 0.0;
    }
    let into_start = (start + zone - position) / zone;
    let into_end = (position - (end - zone)) / zone;
    if into_start > 0.0 {
        -MAX_SPEED * into_start.min(1.0)
    } else if into_end > 0.0 {
        MAX_SPEED * into_end.min(1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::{EDGE_ZONE, MAX_SPEED, edge_speed};

    #[test]
    fn a_drag_scrolls_a_list_only_near_its_edges_and_faster_the_closer_it_gets() {
        let (start, end) = (100.0, 500.0);
        assert_eq!(edge_speed(300.0, start, end), 0.0);
        assert_eq!(edge_speed(start + EDGE_ZONE, start, end), 0.0);
        assert_eq!(edge_speed(start + EDGE_ZONE / 2.0, start, end), -MAX_SPEED / 2.0);
        assert_eq!(edge_speed(start, start, end), -MAX_SPEED);
        assert_eq!(edge_speed(end, start, end), MAX_SPEED);
        // Just past the edge still scrolls; a drag that has left for elsewhere does not.
        assert_eq!(edge_speed(end + 10.0, start, end), MAX_SPEED);
        assert_eq!(edge_speed(end + EDGE_ZONE + 1.0, start, end), 0.0);
        // A list too short for two full zones keeps a dead middle.
        assert_eq!(edge_speed(115.0, 100.0, 130.0), 0.0);
    }
}
