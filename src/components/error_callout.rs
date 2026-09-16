//! The one way an error is drawn inside a view: where it happened, with its details a click away.

use std::rc::Rc;

use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::components::Button;
use crate::error::{ErrorKind, ErrorReport};
use crate::state::{AppState, StatusMessage};
use crate::theme::{borders, spacing};

type CloseHandler = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct ErrorCallout {
    id: SharedString,
    report: ErrorReport,
    compact: bool,
    actions: Vec<AnyElement>,
    on_close: Option<CloseHandler>,
    state: Option<Entity<AppState>>,
}

impl ErrorCallout {
    /// `id` must be stable across renders; it keeps the Details disclosure open.
    pub fn new(id: impl Into<SharedString>, report: ErrorReport) -> Self {
        Self {
            id: id.into(),
            report,
            compact: false,
            actions: Vec::new(),
            on_close: None,
            state: None,
        }
    }

    /// One line that expands on demand, for places where the input must stay visible.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    /// A fix such as Retry or Go to stage. Keep it to one or two.
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    pub fn on_close(mut self, on_close: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_close = Some(Rc::new(on_close));
        self
    }

    /// Enables Copy feedback in the status bar, and Ask AI when the assistant is set up.
    pub fn state(mut self, state: Entity<AppState>) -> Self {
        self.state = Some(state);
        self
    }
}

impl RenderOnce for ErrorCallout {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { id, report, compact, actions, on_close, state } = self;
        let expanded =
            window.use_keyed_state(SharedString::from(format!("{id}-expanded")), cx, |_, _| false);
        let is_expanded = *expanded.read(cx);
        let warning = report.kind == ErrorKind::Validation;
        let tone = if warning { cx.theme().warning } else { cx.theme().danger };
        let icon = if warning { IconName::TriangleAlert } else { IconName::CircleX };
        let has_details = report.details.is_some() || report.server_message.is_some();
        let ask_ai = state.as_ref().is_some_and(|state| state.read(cx).ai_assistant_available());

        let toggle = {
            let expanded = expanded.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                expanded.update(cx, |open, cx| {
                    *open = !*open;
                    cx.notify();
                })
            }
        };

        let container = div()
            .id(id.clone())
            .flex()
            .gap(spacing::sm())
            .w_full()
            .min_w(px(0.0))
            .px(px(10.0))
            .py(px(8.0))
            .rounded(borders::radius_sm())
            .border_1()
            .border_color(tone.opacity(0.35))
            .bg(tone.opacity(0.06))
            .child(Icon::new(icon).small().text_color(tone).mt(px(1.0)).flex_shrink_0());

        if compact && !is_expanded {
            let line = match report.title.is_empty() {
                true => report.message.clone(),
                false => format!("{}: {}", report.title, report.message),
            };
            return container
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .truncate()
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .child(line),
                )
                .child(
                    Button::new(SharedString::from(format!("{id}-more")))
                        .ghost()
                        .xsmall()
                        .label("Details")
                        .on_click(toggle),
                )
                .into_any_element();
        }

        let title = (!report.title.is_empty()).then(|| {
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(tone)
                .child(report.title.clone())
        });
        let code = report.code_label().map(|label| {
            div()
                .px(px(5.0))
                .rounded(borders::radius_xs())
                .bg(cx.theme().muted)
                .text_xs()
                .font_family(crate::theme::fonts::mono())
                .text_color(cx.theme().muted_foreground)
                .child(label)
        });
        let close = on_close.map(|on_close| {
            Button::new(SharedString::from(format!("{id}-close")))
                .ghost()
                .xsmall()
                .icon(Icon::new(IconName::Close))
                .accessibility_label("Dismiss")
                .on_click(move |_, window, cx| on_close(window, cx))
        });

        let details = (is_expanded && has_details).then(|| {
            let mut text = String::new();
            if let Some(server_message) = &report.server_message {
                text.push_str(server_message);
            }
            if let Some(details) = &report.details {
                if !text.is_empty() {
                    text.push_str("\n\n");
                }
                text.push_str(details);
            }
            div()
                .id(SharedString::from(format!("{id}-details")))
                .max_h(px(240.0))
                .overflow_y_scroll()
                .px(spacing::sm())
                .py(px(6.0))
                .rounded(borders::radius_xs())
                .bg(cx.theme().muted.opacity(0.6))
                .font_family(crate::theme::fonts::mono())
                .text_xs()
                .text_color(cx.theme().foreground)
                .whitespace_normal()
                .child(text)
        });

        let copy = {
            let text = report.copy_text();
            let state = state.clone();
            Button::new(SharedString::from(format!("{id}-copy")))
                .ghost()
                .xsmall()
                .icon(Icon::new(IconName::Copy))
                .label("Copy")
                .tooltip("Copy the error and its details")
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    if let Some(state) = &state {
                        state.update(cx, |state, cx| {
                            state.set_status_message(Some(StatusMessage::info(
                                "Copied error details",
                            )));
                            cx.notify();
                        });
                    }
                })
        };
        let ask = state.filter(|_| ask_ai).map(|state| {
            let prompt = ai_prompt(&report);
            Button::new(SharedString::from(format!("{id}-ask-ai")))
                .ghost()
                .xsmall()
                .icon(Icon::new(IconName::Bot))
                .label("Ask AI")
                .tooltip("Ask the AI assistant to explain this error")
                .on_click(move |_, _, cx| {
                    state.update(cx, |state, cx| {
                        state.ask_ai(prompt.clone());
                        cx.notify();
                    });
                })
        });

        container
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .flex_1()
                    .min_w(px(0.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::sm())
                            .min_w(px(0.0))
                            .children(title)
                            .children(code)
                            .child(div().flex_1())
                            .children(close),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().foreground)
                            .when(!is_expanded, |message| message.line_clamp(3))
                            .child(report.message.clone()),
                    )
                    .children(details)
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(2.0))
                            .ml(px(-6.0))
                            .children(actions)
                            .when(has_details || compact, |row| {
                                row.child(
                                    Button::new(SharedString::from(format!("{id}-toggle")))
                                        .ghost()
                                        .xsmall()
                                        .label(if is_expanded { "Hide details" } else { "Details" })
                                        .on_click(toggle),
                                )
                            })
                            .child(copy)
                            .children(ask),
                    ),
            )
            .into_any_element()
    }
}

fn ai_prompt(report: &ErrorReport) -> String {
    format!(
        "Explain this MongoDB error in OpenMango and how to fix it. Keep it short.\n\n{}",
        report.copy_text()
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    fn rust_files(dir: &Path, found: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("readable source directory").flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_files(&path, found);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                found.push(path);
            }
        }
    }

    /// `danger_foreground` is text *on* a danger fill. As text on a normal surface it nearly
    /// matches the background, which made error messages unreadable.
    #[test]
    fn danger_foreground_text_only_sits_on_danger_fills() {
        let mut files = Vec::new();
        rust_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
        // Split so this test's own source doesn't match.
        let (text_color, token) = (concat!("text_", "color("), concat!("danger", "_foreground"));
        let mut offenders = Vec::new();
        for path in files {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let lines: Vec<&str> = text.lines().collect();
            for (ix, line) in lines.iter().enumerate() {
                if !(line.contains(text_color) && line.contains(token)) {
                    continue;
                }
                let on_fill = lines[ix.saturating_sub(4)..=ix]
                    .iter()
                    .any(|line| line.contains(".bg(") && line.contains("theme().danger)"));
                if !on_fill {
                    offenders.push(format!("{}:{}", path.display(), ix + 1));
                }
            }
        }
        assert!(offenders.is_empty(), "use `danger` for error text instead: {offenders:?}");
    }
}
