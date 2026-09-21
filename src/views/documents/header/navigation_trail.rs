//! The trail a tab followed to get here, and the way back along it.
//!
//! Hidden entirely until a tab has navigated: a row that says only where you already are is
//! noise, and the header is dense enough. It takes no vertical space when there is no trail.

use gpui_kit::component::button::{Button as KitButton, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _};
use gpui_kit::*;

use crate::keyboard::{NavigateBack, NavigateForward};
use crate::state::AppState;
use crate::theme::spacing;

/// Crumbs shown before the middle is elided. Four fits a deep chain of references without the
/// row competing with the collection name above it.
const MAX_CRUMBS: usize = 4;

/// Render the active tab's trail, or nothing when it has not moved.
pub fn render_navigation_trail(state: &Entity<AppState>, window: &Window, cx: &App) -> Option<Div> {
    let trail = state.read(cx).navigation_trail();
    if trail.len() < 2 {
        return None;
    }
    let can_back = state.read(cx).can_navigate_back();
    let can_forward = state.read(cx).can_navigate_forward();
    let last = trail.len() - 1;

    let mut row = div()
        .flex()
        .items_center()
        .gap(spacing::xs())
        .child(step_button(
            "trail-back",
            IconName::ChevronLeft,
            "Back",
            &NavigateBack,
            can_back,
            state.clone(),
            window,
            |state, cx| {
                state.update(cx, |state, cx| {
                    state.navigate_back(cx);
                });
            },
        ))
        .child(step_button(
            "trail-forward",
            IconName::ChevronRight,
            "Forward",
            &NavigateForward,
            can_forward,
            state.clone(),
            window,
            |state, cx| {
                state.update(cx, |state, cx| {
                    state.navigate_forward(cx);
                });
            },
        ));

    // Long trails elide the middle rather than the end: where you started and where you are now
    // are the two things worth keeping.
    for (index, key) in trail.iter().enumerate() {
        let elided = trail.len() > MAX_CRUMBS && index > 0 && index < last - 1;
        if elided {
            if index == 1 {
                row = row.child(separator(cx)).child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("… {} more", last - 2)),
                );
            }
            continue;
        }
        if index > 0 {
            row = row.child(separator(cx));
        }
        row = row.child(crumb(index, key.collection.clone(), index == last, state.clone(), cx));
    }

    Some(row)
}

fn separator(cx: &App) -> Div {
    div().text_xs().text_color(cx.theme().muted_foreground).child("›")
}

/// One step of the trail. The last is where you are, so it reads as a label rather than a
/// control: nothing happens if you click it.
fn crumb(
    index: usize,
    collection: String,
    is_current: bool,
    state: Entity<AppState>,
    cx: &App,
) -> AnyElement {
    if is_current {
        return div()
            .text_xs()
            .text_color(cx.theme().foreground)
            .child(collection)
            .into_any_element();
    }
    KitButton::new(("trail-crumb", index))
        .ghost()
        .xsmall()
        .label(collection)
        .on_click(move |_, _window, cx| {
            state.update(cx, |state, cx| {
                state.navigate_to_trail_index(index, cx);
            });
        })
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn step_button(
    id: &'static str,
    icon: IconName,
    label: &'static str,
    action: &dyn Action,
    enabled: bool,
    state: Entity<AppState>,
    window: &Window,
    on_click: fn(&Entity<AppState>, &mut App),
) -> impl IntoElement {
    let tooltip = crate::keyboard::shortcut_label(window, action)
        .map(|shortcut| format!("{label} ({shortcut})"))
        .unwrap_or_else(|| label.to_string());

    KitButton::new(id)
        .ghost()
        .xsmall()
        .icon(Icon::new(icon))
        .tooltip(tooltip)
        .disabled(!enabled)
        .on_click(move |_, _window, cx| on_click(&state, cx))
}
