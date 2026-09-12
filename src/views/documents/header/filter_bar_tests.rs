use gpui_kit::component::RopeExt as _;
use gpui_kit::component::input::EditorState;
use gpui_kit::component::input::InputEvent;
use gpui_kit::component::{ActiveTheme as _, Root, Theme};
use gpui_kit::{
    AppContext as _, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    Modifiers, ParentElement as _, Pixels, Point, Render, Styled as _, TestAppContext,
    VisualTestContext, Window, div, point, px, size,
};

use super::{query_editor, query_find_button};
use crate::views::documents::query_editor::{format_query_editor, new_query_editor};

struct QueryHarness {
    input: Entity<EditorState>,
    width: Pixels,
    expanded: bool,
    option: bool,
    _subscription: gpui_kit::Subscription,
}

impl Render for QueryHarness {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.option {
            return super::super::header_container(cx.theme().background).size_full().child(
                div().w(self.width).debug_selector(|| "option-row".into()).child(
                    super::render_query_segment(
                        "Projection",
                        "All fields",
                        Some(self.input.clone()),
                        true,
                        false,
                        window,
                        cx,
                    ),
                ),
            );
        }
        let rows =
            if self.expanded { 10 } else { self.input.read(cx).text().lines_len().clamp(1, 4) };
        super::super::header_container(cx.theme().background).size_full().child(
            div()
                .flex()
                .items_start()
                .gap_2()
                .w(self.width)
                .child(div().flex_1().min_w(px(0.)).debug_selector(|| "editor".into()).child(
                    query_editor(&self.input, rows, "MongoDB filter", false, false, window, cx),
                ))
                .child(div().debug_selector(|| "find".into()).child(query_find_button(window))),
        )
    }
}

fn harness<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
) -> (Entity<QueryHarness>, Entity<EditorState>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        Theme::global_mut(cx).font_size = px(16.);
    });
    let source = text.to_string();
    let mut view = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let query = cx.new(|cx| {
            let input = cx
                .new(|cx| new_query_editor(window, cx, "Filter documents…").default_value(source));
            let subscription =
                cx.subscribe_in(&input, window, |_, _, _: &InputEvent, _, cx| cx.notify());
            input.update(cx, |input, cx| input.focus(window, cx));
            QueryHarness {
                input,
                width: px(600.),
                expanded: false,
                option: false,
                _subscription: subscription,
            }
        });
        view = Some(query.clone());
        Root::new(query, window, cx).bordered(false)
    });
    let view = view.unwrap();
    let input = view.read_with(cx, |view, _| view.input.clone());
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    draw(cx);
    (view, input, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
}

fn caret_point(
    input: &Entity<EditorState>,
    offset: usize,
    cx: &mut VisualTestContext,
) -> Point<Pixels> {
    input.update(cx, |input, cx| input.set_selected_range(offset..offset, cx));
    draw(cx);
    input.read_with(cx, |input, _| {
        let (caret, _) = input.cursor_layout().expect("laid out caret");
        point(caret.left() + px(0.25), caret.center().y + input.scroll_offset().y)
    })
}

#[gpui_kit::test]
fn query_options_stay_compact_and_open_the_complete_projection(cx: &mut TestAppContext) {
    let projection = "{\n  _id: 1,\n  title: 1,\n  year: 1,\n  rated: 1,\n  runtime: 1,\n  genres: 1,\n  released: 1,\n  imdb: 1\n}";
    let (view, input, cx) = harness(cx, projection);
    for (width, rem) in [(600., 16.), (360., 20.)] {
        view.update(cx, |view, cx| {
            view.option = true;
            view.width = px(width);
            cx.notify();
        });
        cx.update(|window, cx| {
            Theme::global_mut(cx).font_size = px(rem);
            window.refresh();
        });
        draw(cx);
        assert!(
            cx.debug_bounds("option-row").unwrap().size.height <= px(rem * 2.5),
            "a multiline projection must not expand the query toolbar"
        );
        let trigger = cx.debug_bounds("query-option-Projection").unwrap();
        cx.simulate_click(trigger.center(), Modifiers::default());
        draw(cx);
        input.read_with(cx, |input, _| {
            let required_height = input.line_height().unwrap() * input.text().lines_len() as f32;
            assert!(input.input_bounds().size.height >= required_height);
            assert!(input.input_bounds().size.width > px(200.));
            assert_eq!(input.value().as_ref(), projection);
        });
        cx.simulate_keystrokes("escape");
        draw(cx);
    }
}

#[gpui_kit::test]
fn query_control_heights_align_at_compact_widths_and_zoom(cx: &mut TestAppContext) {
    let (view, input, cx) = harness(cx, "{ archived: false }");
    for (width, rem) in [(600., 16.), (320., 16.), (480., 20.)] {
        view.update(cx, |view, cx| {
            view.width = px(width);
            cx.notify();
        });
        cx.update(|window, cx| {
            Theme::global_mut(cx).font_size = px(rem);
            window.refresh();
        });
        draw(cx);
        let editor = cx.debug_bounds("editor").unwrap();
        let find = cx.debug_bounds("find").unwrap();
        assert_eq!(editor.top(), find.top());
        assert_eq!(editor.size.height, find.size.height);
        assert!(editor.size.width > px(100.));
        let line_height = input.read_with(cx, |input, _| input.line_height().unwrap());
        let text = input.read_with(cx, |input, _| input.input_bounds());
        assert!(text.size.height >= line_height, "the line must fit inside the native padding");
        let caret = caret_point(&input, 0, cx);
        assert!(
            (caret.y - editor.center().y).abs() <= px(1.),
            "text and control centers must align: width={width} rem={rem} editor={editor:?} text={text:?} caret={caret:?} line={line_height:?}"
        );
    }
}

#[gpui_kit::test]
fn query_mouse_clicks_keep_the_requested_caret_position(cx: &mut TestAppContext) {
    let (_, input, cx) = harness(cx, "{}");
    for source in ["{}", "{ archived: false }", "{ name: \"ნინო\" }", "ნინო", "🎉"]
    {
        cx.update(|window, cx| input.update(cx, |input, cx| input.set_value(source, window, cx)));
        draw(cx);
        let middle = source.find(':').map(|offset| offset + 1).unwrap_or_else(|| {
            source.char_indices().nth(source.chars().count() / 2).map_or(0, |(ix, _)| ix)
        });
        for target in [0, middle, source.len()] {
            let location = caret_point(&input, target, cx);
            input.update(cx, |input, cx| input.set_selected_range(1..1, cx));
            cx.update(|window, cx| window.blur(cx));
            draw(cx);
            cx.simulate_click(location, Modifiers::default());
            assert_eq!(
                input.read_with(cx, |input, _| input.cursor()),
                target,
                "source={source:?}, click={location:?}, geometry={:?}",
                input.read_with(cx, |input, _| (
                    input.input_bounds(),
                    input.cursor_layout(),
                    input.scroll_offset()
                ))
            );
        }
    }
}

#[gpui_kit::test]
fn collapsed_query_clicks_focus_the_editor_across_the_whole_field(cx: &mut TestAppContext) {
    let (view, input, cx) = harness(cx, "");
    for rem in [12., 13., 14., 15., 16., 18., 20.] {
        cx.update(|window, cx| {
            Theme::global_mut(cx).font_size = px(rem);
            window.refresh();
        });
        for source in ["", "{ archived: false }"] {
            cx.update(|window, cx| {
                input.update(cx, |input, cx| input.set_value(source, window, cx))
            });
            view.update(cx, |view, cx| {
                view.expanded = true;
                cx.notify();
            });
            draw(cx);
            view.update(cx, |view, cx| {
                view.expanded = false;
                cx.notify();
            });
            draw(cx);
            let field = cx.debug_bounds("editor").unwrap();
            let positions = [
                ("center", field.center()),
                ("top padding", point(field.center().x, field.top() + px(3.))),
                ("bottom padding", point(field.center().x, field.bottom() - px(3.))),
                ("leading padding", point(field.left() + px(3.), field.center().y)),
                ("trailing padding", point(field.right() - px(3.), field.center().y)),
            ];
            for (label, position) in positions {
                cx.update(|window, cx| window.blur(cx));
                draw(cx);
                cx.simulate_click(position, Modifiers::default());
                draw(cx);
                let focused =
                    cx.update(|window, cx| input.read(cx).focus_handle(cx).is_focused(window));
                assert!(focused, "{label} must focus the real editor: source={source:?}");
                let (caret, _) = input
                    .read_with(cx, |input, _| input.cursor_layout())
                    .expect("focused editor has a caret");
                assert!(
                    field.contains(&caret.center()),
                    "{label} caret must be visible inside the collapsed field"
                );
                let painted = cx.update(|window, _| {
                    let expected = caret.scale(window.scale_factor());
                    window.painted_quads().into_iter().any(|quad| {
                        (quad.bounds.left() - expected.left()).0.abs() <= 1.0
                            && (quad.bounds.top() - expected.top()).0.abs() <= 1.0
                            && (quad.bounds.size.width - expected.size.width).0.abs() <= 1.0
                            && (quad.bounds.size.height - expected.size.height).0.abs() <= 1.0
                            && quad.bounds.intersect(&quad.content_mask.bounds).size.height
                                >= expected.size.height * 0.8
                    })
                });
                assert!(
                    painted,
                    "{label} must paint an unclipped caret: rem={rem}, source={source:?}, caret={caret:?}, details={:?}",
                    cx.update(|window, _| (
                        window.is_window_active(),
                        window
                            .painted_quads()
                            .into_iter()
                            .filter(|q| q.bounds.size.width.as_f32() < 4.0)
                            .map(|q| (q.bounds, q.content_mask.bounds))
                            .collect::<Vec<_>>()
                    ))
                );
            }
        }
    }
}

#[gpui_kit::test]
fn query_formatting_preserves_caret_and_can_be_undone_once(cx: &mut TestAppContext) {
    let source = r#"{backend:ObjectId("6a3e5059e61da6678f2a5577")}"#;
    let (_, input, cx) = harness(cx, source);
    let cursor = source.find("6a3e").unwrap() + 4;
    input.update(cx, |input, cx| input.set_selected_range(cursor..cursor, cx));
    let formatted = cx.update(|window, cx| format_query_editor(&input, window, cx));
    draw(cx);
    assert_eq!(formatted, r#"{ backend: ObjectId("6a3e5059e61da6678f2a5577") }"#);
    assert_eq!(input.read_with(cx, |input, _| input.cursor()), formatted.find("6a3e").unwrap() + 4,);
    assert!(cx.update(|window, cx| input.read(cx).focus_handle(cx).is_focused(window)));
    // Submitting again must not add an unchanged formatting edit to undo history.
    cx.update(|window, cx| format_query_editor(&input, window, cx));
    cx.update(|window, cx| window.dispatch_action(Box::new(gpui_kit::component::input::Undo), cx));
    draw(cx);
    assert_eq!(input.read_with(cx, |input, _| input.value().to_string()), source);
    assert_eq!(input.read_with(cx, |input, _| input.cursor()), cursor);
}

#[gpui_kit::test]
fn header_query_keeps_focus_after_left_and_right_clicks(cx: &mut TestAppContext) {
    let (view, input, cx) = harness(cx, "{ archived: false }");
    for expanded in [false, true, false] {
        view.update(cx, |view, cx| {
            view.expanded = expanded;
            cx.notify();
        });
        draw(cx);
        let location = caret_point(&input, 3, cx);
        for button in [gpui_kit::MouseButton::Right, gpui_kit::MouseButton::Left] {
            cx.update(|window, cx| window.blur(cx));
            draw(cx);
            cx.simulate_mouse_down(location, button, Modifiers::default());
            cx.simulate_mouse_up(location, button, Modifiers::default());
            draw(cx);
            if button == gpui_kit::MouseButton::Right && !cfg!(target_os = "macos") {
                // Kit's Linux popup takes keyboard focus until it is dismissed.
                cx.simulate_keystrokes("escape");
                draw(cx);
            }
            assert!(
                cx.update(|window, cx| input.read(cx).focus_handle(cx).is_focused(window)),
                "the header must preserve editor focus after {button:?} click; expanded={expanded}"
            );
            assert_eq!(input.read_with(cx, |input, _| input.cursor()), 3);
        }
    }
}

#[gpui_kit::test]
fn collapsed_query_keeps_its_only_line_visible_when_scrolled(cx: &mut TestAppContext) {
    let (_, input, cx) = harness(cx, "{ archived: false }");
    input.update(cx, |input, cx| input.set_scroll_offset(point(px(0.), px(-9999.)), cx));
    draw(cx);
    assert_eq!(
        input.read_with(cx, |input, _| input.scroll_offset().y),
        px(0.),
        "a collapsed one-line query must not scroll into blank editor space"
    );
    let field = cx.debug_bounds("editor").unwrap();
    cx.update(|window, cx| window.blur(cx));
    cx.simulate_click(field.center(), Modifiers::default());
    draw(cx);
    let (caret, _) =
        input.read_with(cx, |input, _| input.cursor_layout()).expect("clicked query has a caret");
    assert!(field.contains(&caret.center()));
}

#[gpui_kit::test]
fn query_shift_click_and_drag_keep_the_selection_anchor(cx: &mut TestAppContext) {
    let (_, input, cx) = harness(cx, "{ archived: false }");
    let end = input.read_with(cx, |input, _| input.value().len());
    let last = caret_point(&input, end - 1, cx);
    for anchor in [0, end] {
        input.update(cx, |input, cx| input.set_selected_range(anchor..anchor, cx));
        draw(cx);
        cx.simulate_click(last, Modifiers { shift: true, ..Default::default() });
        assert_eq!(input.read_with(cx, |input, _| input.cursor()), end - 1);
        assert_eq!(
            input.read_with(cx, |input, _| input.selected_range()),
            anchor.min(end - 1)..anchor.max(end - 1)
        );
    }
    let earlier = caret_point(&input, 2, cx);
    cx.simulate_mouse_down(last, gpui_kit::MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(earlier, gpui_kit::MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(earlier, gpui_kit::MouseButton::Left, Modifiers::default());
    assert_eq!(input.read_with(cx, |input, _| input.selected_range()), 2..end - 1);
}

#[gpui_kit::test]
fn query_horizontal_click_uses_the_rendered_glyph_position(cx: &mut TestAppContext) {
    let source = format!("{{ description: \"{}\" }}", "a long value ".repeat(40));
    let (_, input, cx) = harness(cx, &source);
    let target = source.len() - 1;
    input.update(cx, |input, cx| {
        input.set_selected_range(target..target, cx);
        input.set_scroll_offset(point(px(-9999.), px(0.)), cx);
    });
    draw(cx);
    let location = caret_point(&input, target, cx);
    assert!(input.read_with(cx, |input, _| input.scroll_offset().x) < px(0.));
    cx.simulate_click(location, Modifiers::default());
    assert_eq!(input.read_with(cx, |input, _| input.cursor()), target);
}

#[gpui_kit::test]
fn query_multiline_clicks_follow_the_scrolled_text(cx: &mut TestAppContext) {
    let (_, input, cx) =
        harness(cx, "{\n  first: 1,\n  second: 2,\n  third: 3,\n  fourth: 4,\n  last: 5\n}");
    cx.simulate_resize(size(px(700.), px(300.)));
    let target = input.read_with(cx, |input, _| input.value().find("last").unwrap());
    input.update(cx, |input, cx| {
        input.set_selected_range(target..target, cx);
        input.set_scroll_offset(point(px(0.), px(-9999.)), cx);
    });
    draw(cx);
    let location = caret_point(&input, target, cx);
    assert!(input.read_with(cx, |input, _| input.scroll_offset().y) < px(0.));
    cx.simulate_click(location, Modifiers::default());
    assert_eq!(input.read_with(cx, |input, _| input.cursor()), target);
}
