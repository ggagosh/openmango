//! Operator picker and pipeline import dialogs.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::components::{Button, cancel_button};
use crate::state::app_state::{SessionKey, parse_pipeline_text};
use crate::state::{AppState, StatusMessage};
use crate::theme::spacing;

use super::super::operators::OPERATOR_GROUPS;

#[derive(Clone, Copy)]
pub(in crate::views::documents) enum OperatorPick {
    /// Insert a new stage at this index.
    Insert(usize),
    /// Change the operator of the stage at this index.
    Replace(usize),
}

pub(in crate::views::documents) fn open_operator_picker(
    window: &mut Window,
    cx: &mut App,
    state: Entity<AppState>,
    session_key: SessionKey,
    pick: OperatorPick,
) {
    let current = match pick {
        OperatorPick::Replace(index) => state
            .read(cx)
            .session(&session_key)
            .and_then(|session| session.data.aggregation.stages.get(index))
            .map(|stage| stage.operator.clone()),
        OperatorPick::Insert(_) => None,
    };
    let title = match pick {
        OperatorPick::Insert(_) => "Add stage",
        OperatorPick::Replace(_) => "Change operator",
    };
    let command = cx.new(|cx| CommandState::new(window, cx));
    window.defer(cx, {
        let command = command.clone();
        move |window, cx| command.update(cx, |command, cx| command.focus(window, cx))
    });

    window.open_dialog(cx, move |dialog: Dialog, _window, _cx| {
        let mut picker = Command::new(&command)
            .placeholder("Search operators")
            .max_h(px(380.0))
            .bordered(false)
            .empty(|state, _, cx| {
                div()
                    .py_6()
                    .w_full()
                    .text_center()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("No operator matches “{}”", state.query(cx)))
            })
            .on_confirm({
                let state = state.clone();
                let session_key = session_key.clone();
                move |index, window, cx| {
                    let Some((operator, _)) = OPERATOR_GROUPS
                        .get(index.section)
                        .and_then(|group| group.operators.get(index.row))
                    else {
                        return;
                    };
                    state.update(cx, |state, cx| {
                        match pick {
                            OperatorPick::Insert(at) => {
                                state.insert_pipeline_stage(&session_key, at, *operator);
                            }
                            OperatorPick::Replace(at) => {
                                state.set_pipeline_stage_operator(
                                    &session_key,
                                    at,
                                    operator.to_string(),
                                );
                            }
                        }
                        cx.notify();
                    });
                    window.close_dialog(cx);
                }
            })
            .on_cancel(|window, cx| window.close_dialog(cx));

        for group in OPERATOR_GROUPS {
            let items = group.operators.iter().map(|&(operator, description)| {
                CommandItem::new()
                    .label(operator)
                    .keywords([operator.trim_start_matches('$'), description, group.label])
                    .checked(current.as_deref() == Some(operator))
                    .child(move |_, cx| {
                        div()
                            .flex()
                            .items_center()
                            .flex_1()
                            .min_w_0()
                            .gap(spacing::sm())
                            .child(div().flex_none().child(operator))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(description),
                            )
                    })
            });
            picker = picker.group(CommandGroup::new().label(group.label).items(items));
        }

        dialog.title(title).w(px(520.0)).child(picker)
    });
}

pub(in crate::views::documents) fn open_import_pipeline_dialog(
    window: &mut Window,
    cx: &mut App,
    state: Entity<AppState>,
    session_key: SessionKey,
) {
    let editor = cx.new(|cx| {
        EditorState::new(window, cx)
            .language("javascript")
            .line_number(true)
            .soft_wrap(true)
            .placeholder("[\n  { $match: { status: \"active\" } }\n]")
    });
    let error = cx.new(|_| None::<String>);
    window.defer(cx, {
        let editor = editor.clone();
        move |window, cx| {
            let focus = editor.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
    });

    window.open_dialog(cx, move |dialog: Dialog, _window, cx| {
        let error_text = error.read(cx).clone();
        let paste = {
            let editor = editor.clone();
            let error = error.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
                    set_error(
                        &error,
                        "The clipboard has no text. Copy the pipeline and try again.",
                        cx,
                    );
                    return;
                };
                error.update(cx, |error, _| *error = None);
                editor.update(cx, |editor, cx| editor.set_value(text, window, cx));
            }
        };
        let import = {
            let editor = editor.clone();
            let error = error.clone();
            let state = state.clone();
            let session_key = session_key.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                let raw = editor.read(cx).value().to_string();
                match parse_pipeline_text(&raw) {
                    Ok(stages) if stages.is_empty() => {
                        set_error(&error, "The pipeline has no stages. Add at least one stage.", cx)
                    }
                    Ok(stages) => {
                        state.update(cx, |state, cx| {
                            state.replace_pipeline_stages(&session_key, stages);
                            state.set_status_message(Some(StatusMessage::info(IMPORTED)));
                            cx.notify();
                        });
                        window.close_dialog(cx);
                    }
                    Err(message) => set_error(&error, message, cx),
                }
            }
        };

        let intro = "Paste a pipeline array. It replaces the current stages.";
        dialog.title("Import pipeline").min_w(px(720.0)).child(
            div()
                .flex()
                .flex_col()
                .gap(spacing::md())
                .p(spacing::md())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(spacing::md())
                        .child(div().text_sm().text_color(cx.theme().muted_foreground).child(intro))
                        .child(
                            Button::new("agg-import-paste")
                                .xsmall()
                                .label("Paste from clipboard")
                                .on_click(paste),
                        ),
                )
                .child(Editor::new(&editor).font_family(crate::theme::fonts::mono()).h(px(320.0)))
                .when_some(error_text, |this, message| {
                    this.child(Alert::error("agg-import-error", message).title("Unable to import"))
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(spacing::xs())
                        .child(cancel_button("agg-import-cancel"))
                        .child(
                            Button::new("agg-import-confirm")
                                .primary()
                                .label("Import")
                                .on_click(import),
                        ),
                ),
        )
    });
}

const IMPORTED: &str = "Pipeline imported. Undo with ⌘Z in the stage list.";

fn set_error(error: &Entity<Option<String>>, message: impl Into<String>, cx: &mut App) {
    let message = message.into();
    error.update(cx, |error, cx| {
        *error = Some(message);
        cx.notify();
    });
}
