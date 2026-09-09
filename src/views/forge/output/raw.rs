use std::ops::Range;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::RopeExt;
use gpui_kit::component::input::{InputEvent, Rope, Textarea, TextareaState};
use gpui_kit::*;

use super::super::ForgeView;
use crate::helpers::auto_pair::diff_ranges;
use crate::theme::fonts;

pub struct RawOutputState {
    pub input: Option<Entity<TextareaState>>,
    pub dirty: bool,
    pub reset_history: bool,
    text: String,
    rows: usize,
    cleared: bool,
    follow: FollowOutput,
    _subscriptions: Vec<Subscription>,
}

impl Default for RawOutputState {
    fn default() -> Self {
        Self {
            input: None,
            dirty: true,
            reset_history: false,
            text: String::new(),
            rows: 1,
            cleared: false,
            follow: FollowOutput::default(),
            _subscriptions: Vec::new(),
        }
    }
}

impl RawOutputState {
    pub fn following(&self) -> bool {
        self.follow.enabled
    }

    pub fn clear(&mut self) {
        self.dirty = true;
        self.reset_history = true;
        self.cleared = true;
        self.follow = FollowOutput::default();
    }
}

#[derive(Debug, PartialEq, Eq)]
enum FollowScroll {
    None,
    Tail,
    Preserve,
}

struct FollowOutput {
    enabled: bool,
    pending: bool,
    last_y: f32,
    height: f32,
    line_height: f32,
}

impl Default for FollowOutput {
    fn default() -> Self {
        Self { enabled: true, pending: false, last_y: 0.0, height: 0.0, line_height: 0.0 }
    }
}

impl FollowOutput {
    fn measure(
        &mut self,
        y: f32,
        height: f32,
        line_height: f32,
        rows: usize,
        selected: bool,
    ) -> FollowScroll {
        let geometry_changed = height != self.height || line_height != self.line_height;
        let bottom = (height - rows as f32 * line_height).min(0.0);
        let at_bottom = y <= bottom + 2.0;
        let moved_up = y > self.last_y + 2.0 && !at_bottom;
        self.last_y = y;
        self.height = height;
        self.line_height = line_height;
        if selected || moved_up {
            let cancel_pending = self.pending;
            self.enabled = false;
            self.pending = false;
            return if cancel_pending { FollowScroll::Preserve } else { FollowScroll::None };
        }
        if at_bottom {
            self.enabled = true;
            self.pending = false;
        } else if self.enabled && geometry_changed {
            self.pending = true;
            return FollowScroll::Tail;
        } else if !self.pending {
            self.enabled = false;
        }
        FollowScroll::None
    }
}

fn map_output_offset(offset: usize, replaced: &Range<usize>, inserted_len: usize) -> usize {
    if offset <= replaced.start {
        offset
    } else if offset >= replaced.end {
        replaced.start + inserted_len + offset - replaced.end
    } else {
        replaced.start
    }
}

fn preserve_output_scroll(
    previous: &Rope,
    current: &Rope,
    replaced: &Range<usize>,
    inserted_len: usize,
    y: f32,
    line_height: f32,
) -> f32 {
    if line_height <= 0.0 {
        return y;
    }
    let row = (-y / line_height).max(0.0).floor() as usize;
    let anchor = previous.line_start_offset(row.min(previous.lines_len().saturating_sub(1)));
    let mapped = if replaced.is_empty() && anchor == replaced.start {
        anchor + inserted_len
    } else {
        map_output_offset(anchor, replaced, inserted_len)
    }
    .min(current.len());
    let next_row = current.offset_to_position(mapped).line;
    y + (row as f32 - next_row as f32) * line_height
}

impl ForgeView {
    pub fn console_has_focus(&self, window: &Window, cx: &App) -> bool {
        self.state
            .output
            .raw
            .input
            .as_ref()
            .is_some_and(|input| input.read(cx).focus_handle(cx).is_focused(window))
    }

    pub fn ensure_raw_output_state(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextareaState> {
        if let Some(state) = &self.state.output.raw.input {
            return state.clone();
        }
        let raw_state = cx.new(|cx| {
            TextareaState::new(window, cx)
                .searchable(true)
                .soft_wrap(false)
                .scroll_beyond_last_line(None)
                .placeholder("No output yet.")
        });
        let subscription = cx.observe(&raw_state, |this, input, cx| {
            let (offset, height, line_height, selected) = {
                let input = input.read(cx);
                (
                    input.scroll_offset(),
                    input.input_bounds().size.height,
                    input.line_height(),
                    !input.selected_range().is_empty(),
                )
            };
            let Some(line_height) = line_height else {
                return;
            };
            if height <= px(0.0) {
                return;
            }
            let raw = &mut this.state.output.raw;
            let was_following = raw.follow.enabled;
            let action = raw.follow.measure(
                offset.y.into(),
                height.into(),
                line_height.into(),
                raw.rows,
                selected,
            );
            match action {
                FollowScroll::Tail => {
                    let target = point(offset.x, -(line_height * raw.rows));
                    input.update(cx, |input, cx| input.set_scroll_offset(target, cx));
                }
                FollowScroll::Preserve => {
                    input.update(cx, |input, cx| input.set_scroll_offset(offset, cx));
                }
                FollowScroll::None => {}
            }
            if was_following != raw.follow.enabled {
                if !raw.follow.enabled {
                    this.state.output.auto_select_results = false;
                }
                cx.notify();
            }
        });
        self.state.output.raw.input = Some(raw_state.clone());
        let focus_subscription = cx.subscribe(&raw_state, |this, _, event, _| {
            if matches!(event, InputEvent::Focus) {
                this.state.output.auto_select_results = false;
            }
        });
        self.state.output.raw._subscriptions = vec![subscription, focus_subscription];
        raw_state
    }
}

fn build_raw_output_text(runs: &[super::super::types::ForgeRunOutput]) -> String {
    let mut out = String::new();
    for (idx, run) in runs.iter().enumerate() {
        let time = run.started_at.with_timezone(&chrono::Local).format("%H:%M:%S");
        let header = if run.id == super::super::types::SYSTEM_RUN_ID {
            format!("[{time}] {}", run.code_preview)
        } else {
            format!("[{time}] Run #{} - {}", run.id, run.code_preview)
        };
        out.push_str(&header);
        out.push('\n');
        // Final values follow prints even if foreground callbacks arrive out of order.
        for line in run.raw_lines.iter().chain(&run.evaluation_lines) {
            out.push_str(line);
            out.push('\n');
        }
        if let Some(err) = &run.error {
            out.push_str(err);
            out.push('\n');
        }
        if idx + 1 < runs.len() {
            out.push('\n');
        }
    }
    out
}

impl ForgeView {
    pub fn sync_raw_output(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.state.output.raw.dirty {
            return;
        }
        let input = self.ensure_raw_output_state(window, cx);
        let text = build_raw_output_text(&self.state.output.output_runs);
        let raw = &mut self.state.output.raw;
        raw.dirty = false;
        let Some((old_range, new_range)) = diff_ranges(&raw.text, &text) else {
            raw.cleared = false;
            raw.reset_history = false;
            return;
        };
        let (selection, cursor, scroll, line_height, previous) = {
            let input = input.read(cx);
            (
                input.selected_range(),
                input.cursor(),
                input.scroll_offset(),
                input.line_height(),
                input.text().clone(),
            )
        };
        let following = raw.cleared || (raw.follow.enabled && selection.is_empty());
        let inserted = &text[new_range];
        let anchor = if cursor == selection.start { selection.end } else { selection.start };
        let next_anchor = map_output_offset(anchor, &old_range, inserted.len());
        let next_cursor = map_output_offset(cursor, &old_range, inserted.len());
        raw.follow.enabled = following;
        raw.follow.pending = following;
        input.update(cx, |input, cx| {
            if raw.reset_history {
                // Discard readonly undo history when retention removes old output.
                input.set_value(text.clone(), window, cx);
            } else {
                input.set_selected_range(old_range.clone(), cx);
                input.replace(inserted.to_string(), window, cx);
            }
            raw.rows = input.text().lines_len();
            if following {
                input.set_selected_range(text.len()..text.len(), cx);
            } else {
                input.set_selected_range(next_anchor..next_cursor, cx);
            }
            let y = if following {
                line_height.map(|height| -(height * raw.rows)).unwrap_or(scroll.y)
            } else {
                px(preserve_output_scroll(
                    &previous,
                    input.text(),
                    &old_range,
                    inserted.len(),
                    scroll.y.into(),
                    line_height.map(f32::from).unwrap_or(0.0),
                ))
            };
            input.set_scroll_offset(point(scroll.x, y), cx);
        });
        // set_value clears the native offset immediately during retention/clear.
        // That programmatic reset must not look like the user scrolling upward.
        raw.follow.last_y = input.read(cx).scroll_offset().y.into();
        raw.text = text;
        raw.cleared = false;
        raw.reset_history = false;
    }

    pub fn follow_raw_output(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.output.auto_select_results = false;
        let input = self.ensure_raw_output_state(window, cx);
        self.state.output.raw.follow.enabled = true;
        self.sync_raw_output(window, cx);
        self.state.output.raw.follow.enabled = true;
        self.state.output.raw.follow.pending = true;
        input.update(cx, |input, cx| {
            let end = input.text().len();
            let scroll = input.scroll_offset();
            input.set_selected_range(end..end, cx);
            if let Some(height) = input.line_height() {
                input.set_scroll_offset(point(scroll.x, -(height * input.text().lines_len())), cx);
            }
        });
        cx.notify();
    }

    pub fn render_raw_output_body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let state = self.ensure_raw_output_state(window, cx);
        self.sync_raw_output(window, cx);
        Textarea::new(&state)
            .readonly(true)
            .h_full()
            .appearance(false)
            .bordered(false)
            .font_family(fonts::mono())
            .text_xs()
            .text_color(cx.theme().secondary_foreground)
    }
}

#[cfg(test)]
mod tests {
    use super::{FollowOutput, FollowScroll, Rope, build_raw_output_text, preserve_output_scroll};
    use crate::helpers::auto_pair::diff_ranges;
    use crate::views::forge::types::ForgeRunOutput;

    #[test]
    fn following_survives_pending_layout_and_pauses_for_reading() {
        let mut follow = FollowOutput::default();
        follow.measure(-100.0, 100.0, 10.0, 20, false);
        follow.pending = true;
        assert_eq!(follow.measure(-100.0, 100.0, 10.0, 30, false), FollowScroll::None);
        assert!(follow.enabled);
        assert_eq!(follow.measure(-80.0, 100.0, 10.0, 30, false), FollowScroll::Preserve);
        assert!(!follow.enabled);
        follow.measure(-80.0, 100.0, 10.0, 40, false);
        assert!(!follow.enabled);
        follow.measure(-300.0, 100.0, 10.0, 40, false);
        assert!(follow.enabled);
    }

    #[test]
    fn resize_follows_and_selection_pauses() {
        let mut follow = FollowOutput::default();
        follow.measure(-100.0, 100.0, 10.0, 20, false);
        assert_eq!(follow.measure(-100.0, 80.0, 10.0, 20, false), FollowScroll::Tail);
        assert_eq!(follow.measure(-100.0, 80.0, 10.0, 20, true), FollowScroll::Preserve);
        assert!(!follow.enabled);
    }

    #[test]
    fn retention_keeps_the_same_output_line_visible() {
        let before = "a\nb\nc\nd\n";
        let after = "c\nd\n";
        let (removed, inserted) = diff_ranges(before, after).unwrap();
        assert_eq!(
            preserve_output_scroll(
                &Rope::from(before),
                &Rope::from(after),
                &removed,
                inserted.len(),
                -30.0,
                10.0
            ),
            -10.0
        );
    }

    #[test]
    fn late_prints_stay_before_the_final_value() {
        let mut run = ForgeRunOutput {
            id: 1,
            started_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            code_preview: "print(1); 2".to_string(),
            raw_lines: Vec::new(),
            evaluation_lines: vec!["2".to_string()],
            error: None,
            last_print_line: None,
        };
        run.raw_lines.extend(["1".to_string(), "late print".to_string()]);
        assert!(build_raw_output_text(&[run]).ends_with("1\nlate print\n2\n"));
    }

    #[test]
    fn late_prints_preserve_the_value_being_read() {
        let before = "header\nresult\n";
        let after = "header\nprint\nresult\n";
        let (removed, inserted) = diff_ranges(before, after).unwrap();
        assert_eq!(
            preserve_output_scroll(
                &Rope::from(before),
                &Rope::from(after),
                &removed,
                inserted.len(),
                -10.0,
                10.0
            ),
            -20.0
        );
    }
}
