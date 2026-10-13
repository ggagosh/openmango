//! The Custom… connection color: a saturation and brightness square over a hue strip, and the
//! hex value, as in the macOS and Figma pickers. Every change applies to the connection at once.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{Icon, Selectable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::components::Button;
use crate::models::ConnectionColor;
use crate::theme::{colors, fonts, spacing};

use super::ConnectionManager;

const HANDLE: Pixels = px(14.0);

pub(super) struct CustomColor {
    /// Hue, saturation and brightness, each in `0..=1`. Kept rather than derived from the color,
    /// so the hue stays put while the color is dragged through gray or black.
    hsv: [f32; 3],
    hex: Entity<InputState>,
    square: Rc<Cell<Bounds<Pixels>>>,
    strip: Rc<Cell<Bounds<Pixels>>>,
}

/// What is being dragged: the square's handle or the strip's.
#[derive(Clone, Copy, PartialEq)]
enum Drag {
    Square,
    Hue,
}

impl CustomColor {
    pub(super) fn new(window: &mut Window, cx: &mut Context<ConnectionManager>) -> Self {
        let hex = cx.new(|cx| InputState::new(window, cx).placeholder("#RRGGBB"));
        cx.subscribe_in(&hex, window, |this, hex, event: &InputEvent, _, cx| {
            if !matches!(event, InputEvent::Change) {
                return;
            }
            let text = hex.read(cx).value();
            let digits = text.trim().trim_start_matches('#');
            let Some(rgb) = u32::from_str_radix(digits, 16).ok().filter(|_| digits.len() == 6)
            else {
                return;
            };
            // Our own writes to the field come back here; they change nothing.
            if this.draft.color != Some(ConnectionColor::Custom(rgb)) {
                this.custom_color.hsv = rgb_to_hsv(rgb, this.custom_color.hsv[0]);
                this.draft.color = Some(ConnectionColor::Custom(rgb));
                cx.notify();
            }
        })
        .detach();
        Self { hsv: [0.0, 0.0, 0.0], hex, square: Rc::default(), strip: Rc::default() }
    }
}

impl ConnectionManager {
    fn set_custom_hsv(&mut self, hsv: [f32; 3], window: &mut Window, cx: &mut Context<Self>) {
        let rgb = hsv_to_rgb(hsv);
        self.custom_color.hsv = hsv;
        self.draft.color = Some(ConnectionColor::Custom(rgb));
        self.custom_color
            .hex
            .update(cx, |hex, cx| hex.set_value(format!("#{rgb:06X}"), window, cx));
        cx.notify();
    }

    /// Opens on the connection's color, so Custom… starts as a tweak of it.
    fn sync_custom_color(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rgb = match self.draft.color {
            Some(ConnectionColor::Custom(rgb)) => rgb,
            Some(color) => colors::rgb_of(colors::connection_accent(color, cx)),
            None => 0x7C3AED,
        };
        self.custom_color.hsv = rgb_to_hsv(rgb, self.custom_color.hsv[0]);
        self.custom_color
            .hex
            .update(cx, |hex, cx| hex.set_value(format!("#{rgb:06X}"), window, cx));
    }

    fn drag_custom_color(
        &mut self,
        drag: Drag,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let x = ((position.x - bounds.left()) / bounds.size.width).clamp(0.0, 1.0);
        let y = ((position.y - bounds.top()) / bounds.size.height).clamp(0.0, 1.0);
        let [hue, saturation, value] = self.custom_color.hsv;
        let hsv = match drag {
            Drag::Square => [hue, x, 1.0 - y],
            Drag::Hue => [x, saturation, value],
        };
        self.set_custom_hsv(hsv, window, cx);
    }

    pub(super) fn render_custom_color_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity();
        let selected = matches!(self.draft.color, Some(ConnectionColor::Custom(_)));
        Popover::new("connection-custom-color")
            .anchor(Anchor::BottomLeft)
            .on_open_change({
                let view = view.clone();
                move |open, window, cx| {
                    if *open {
                        view.update(cx, |this, cx| this.sync_custom_color(window, cx));
                    }
                }
            })
            .trigger(
                Button::new("connection-custom-color-trigger")
                    .ghost()
                    .small()
                    .icon(Icon::new(gpui_kit::assets::IconName::Palette))
                    .selected(selected)
                    .tooltip("Custom color…"),
            )
            .content(move |_, _, cx| custom_color_panel(&view, cx))
            .into_any_element()
    }
}

fn custom_color_panel(view: &Entity<ConnectionManager>, cx: &mut App) -> AnyElement {
    let custom = &view.read(cx).custom_color;
    let [hue, saturation, value] = custom.hsv;
    let (square, strip, hex) = (custom.square.clone(), custom.strip.clone(), custom.hex.clone());
    let color: Hsla = rgb(hsv_to_rgb(custom.hsv)).into();
    let pure_hue = hsla(hue, 1.0, 0.5, 1.0);
    let handle = |border: Hsla| {
        div()
            .absolute()
            .size(HANDLE)
            .ml(-HANDLE / 2.0)
            .mt(-HANDLE / 2.0)
            .rounded_full()
            .border_2()
            .border_color(border)
            .shadow_sm()
    };
    // The bounds are recorded as the frame lays out, for the pointer to be mapped onto.
    let record = |bounds: Rc<Cell<Bounds<Pixels>>>| {
        canvas(move |laid_out, _, _| bounds.set(laid_out), |_, _, _, _| {}).absolute().size_full()
    };
    let draggable = |id: &'static str, drag: Drag, bounds: Rc<Cell<Bounds<Pixels>>>| {
        let (down_view, move_view) = (view.clone(), view.clone());
        div()
            .id(id)
            .relative()
            .cursor_crosshair()
            .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                let bounds = bounds.get();
                down_view.update(cx, |this, cx| {
                    this.drag_custom_color(drag, event.position, bounds, window, cx)
                });
            })
            .on_drag(drag, |_, _, _, cx| cx.new(|_| Empty))
            .on_drag_move(move |event: &DragMoveEvent<Drag>, window, cx| {
                if *event.drag(cx) != drag {
                    return;
                }
                move_view.update(cx, |this, cx| {
                    this.drag_custom_color(drag, event.event.position, event.bounds, window, cx)
                });
            })
    };

    div()
        .w(px(224.0))
        .flex()
        .flex_col()
        .gap(spacing::sm())
        .child(
            draggable("custom-color-square", Drag::Square, square.clone())
                .h(px(150.0))
                .rounded(px(6.0))
                .bg(linear_gradient(
                    90.0,
                    linear_color_stop(white(), 0.0),
                    linear_color_stop(pure_hue, 1.0),
                ))
                .child(div().absolute().size_full().rounded(px(6.0)).bg(linear_gradient(
                    180.0,
                    linear_color_stop(black().opacity(0.0), 0.0),
                    linear_color_stop(black(), 1.0),
                )))
                .child(record(square))
                .child(
                    handle(white()).left(relative(saturation)).top(relative(1.0 - value)).bg(color),
                ),
        )
        .child(
            draggable("custom-color-hue", Drag::Hue, strip.clone())
                .h(px(12.0))
                .flex()
                // Six two-stop gradients through red, yellow, green, cyan, blue, magenta, red.
                .children((0..6).map(|segment| {
                    let at = |offset: f32| hsla((segment as f32 + offset) / 6.0, 1.0, 0.5, 1.0);
                    div()
                        .flex_1()
                        .h_full()
                        .when(segment == 0, |part| part.rounded_l_full())
                        .when(segment == 5, |part| part.rounded_r_full())
                        .bg(linear_gradient(
                            90.0,
                            linear_color_stop(at(0.0), 0.0),
                            linear_color_stop(at(1.0), 1.0),
                        ))
                }))
                .child(record(strip))
                .child(handle(white()).left(relative(hue)).top(relative(0.5)).bg(pure_hue)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(spacing::sm())
                .child(
                    div()
                        .size(px(22.0))
                        .flex_shrink_0()
                        .rounded_full()
                        .bg(color)
                        .border_1()
                        .border_color(cx.theme().border),
                )
                .child(Input::new(&hex).small().font_family(fonts::mono())),
        )
        .into_any_element()
}

/// `[hue, saturation, value]`, each in `0..=1`, to `0xRRGGBB`.
fn hsv_to_rgb([hue, saturation, value]: [f32; 3]) -> u32 {
    let channel = |n: f32| {
        let k = (n + hue * 6.0) % 6.0;
        let level = value - value * saturation * k.min(4.0 - k).clamp(0.0, 1.0);
        (level * 255.0).round() as u32
    };
    channel(5.0) << 16 | channel(3.0) << 8 | channel(1.0)
}

/// `0xRRGGBB` to `[hue, saturation, value]`. A gray has no hue of its own and keeps `hue`.
fn rgb_to_hsv(rgb: u32, hue: f32) -> [f32; 3] {
    let [r, g, b] = [16, 8, 0].map(|shift| ((rgb >> shift) & 0xff) as f32 / 255.0);
    let max = r.max(g).max(b);
    let range = max - r.min(g).min(b);
    let hue = if range == 0.0 {
        hue
    } else if max == r {
        ((g - b) / range).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / range + 2.0) / 6.0
    } else {
        ((r - g) / range + 4.0) / 6.0
    };
    [hue, if max == 0.0 { 0.0 } else { range / max }, max]
}

#[cfg(test)]
mod tests {
    #[test]
    fn hsv_and_rgb_convert_both_ways() {
        assert_eq!(super::hsv_to_rgb([0.0, 1.0, 1.0]), 0xFF0000);
        assert_eq!(super::hsv_to_rgb([1.0 / 3.0, 1.0, 1.0]), 0x00FF00);
        assert_eq!(super::hsv_to_rgb([0.5, 0.0, 0.5]), 0x808080);
        for rgb in [0x000000, 0xFFFFFF, 0x7C3AED, 0xCA8A04, 0x01FF80, 0xF47B74, 0x123456] {
            assert_eq!(super::hsv_to_rgb(super::rgb_to_hsv(rgb, 0.0)), rgb, "{rgb:06X}");
        }
        // Gray keeps the hue it was dragged from.
        assert_eq!(super::rgb_to_hsv(0x808080, 0.3)[0], 0.3);
    }
}
