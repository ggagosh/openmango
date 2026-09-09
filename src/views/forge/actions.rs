use gpui_kit::component::input;
use gpui_kit::{Context, Div, Entity, InteractiveElement};

use crate::components::request_connection_write;
use crate::state::AppState;

use crate::keyboard::{
    AcceptForgeCompletion, CancelForgeRun, ClearForgeOutput, DeleteForgeWordBackward,
    DeleteForgeWordForward, FindInForgeOutput, FocusForgeEditor, FocusForgeOutput,
    InsertForgeNewline, MoveForgeWordBackward, MoveForgeWordForward, NextForgeCompletion,
    PreviousForgeCompletion, RunForgeAll, RunForgeSelectionOrStatement, SelectForgeWordBackward,
    SelectForgeWordForward, TriggerForgeCompletion,
};

use super::ForgeView;
use super::editor_behavior::WordAction;

pub fn bind_root_actions(
    root: Div,
    app_state: Entity<AppState>,
    cx: &mut Context<ForgeView>,
) -> Div {
    let view = cx.entity();
    let selection_app_state = app_state.clone();
    let selection_view = view.clone();
    // These callbacks can run immediately after the write check, so the action
    // handler must not hold the ForgeView update lease that cx.listener takes.
    root
        // These native commands must never be mistaken for a completion acceptance.
        .capture_action(cx.listener(|this, _: &input::Paste, window, cx| {
            this.dismiss_completions(window, cx);
            cx.propagate();
        }))
        .capture_action(cx.listener(|this, _: &input::Cut, window, cx| {
            this.dismiss_completions(window, cx);
            cx.propagate();
        }))
        .capture_action(cx.listener(|this, _: &input::Undo, window, cx| {
            if this.console_has_focus(window, cx) {
                cx.stop_propagation();
                return;
            }
            this.dismiss_completions(window, cx);
            cx.propagate();
        }))
        .capture_action(cx.listener(|this, _: &input::Redo, window, cx| {
            if this.console_has_focus(window, cx) {
                cx.stop_propagation();
                return;
            }
            this.dismiss_completions(window, cx);
            cx.propagate();
        }))
        .capture_action(cx.listener(|this, _: &input::Escape, window, cx| {
            if this.dismiss_completions(window, cx) {
                cx.stop_propagation();
            } else {
                cx.propagate();
            }
        }))
        .on_action(move |_: &RunForgeAll, window, cx| {
            let Some(key) = app_state.read(cx).active_forge_tab_key().cloned() else {
                return;
            };
            let view = view.clone();
            request_connection_write(
                app_state.clone(),
                crate::components::WriteRequest::new(
                    key.connection_id,
                    key.database,
                    "Run Forge code that may write to MongoDB",
                    None,
                ),
                window,
                cx,
                move |window, cx| {
                    view.update(cx, |view, cx| {
                        super::controller::ForgeController::run_all(view, cx);
                        super::controller::ForgeController::focus_editor(view, window, cx);
                    });
                },
            );
            cx.stop_propagation();
        })
        .on_action(move |_: &RunForgeSelectionOrStatement, window, cx| {
            let Some(key) = selection_app_state.read(cx).active_forge_tab_key().cloned() else {
                return;
            };
            let view = selection_view.clone();
            request_connection_write(
                selection_app_state.clone(),
                crate::components::WriteRequest::new(
                    key.connection_id,
                    key.database,
                    "Run Forge code that may write to MongoDB",
                    None,
                ),
                window,
                cx,
                move |window, cx| {
                    view.update(cx, |view, cx| {
                        super::controller::ForgeController::run_selection_or_statement(
                            view, window, cx,
                        );
                        super::controller::ForgeController::focus_editor(view, window, cx);
                    });
                },
            );
            cx.stop_propagation();
        })
        .on_action(cx.listener(|this, _: &CancelForgeRun, _window, cx| {
            super::controller::ForgeController::cancel_run(this, cx);
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &ClearForgeOutput, _window, cx| {
            super::controller::ForgeController::clear_output(this, _window, cx);
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &FocusForgeEditor, window, cx| {
            super::controller::ForgeController::focus_editor(this, window, cx);
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &FocusForgeOutput, window, cx| {
            super::controller::ForgeController::focus_output(this, window, cx);
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &AcceptForgeCompletion, window, cx| {
            this.accept_completion_or_indent(window, cx);
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &TriggerForgeCompletion, window, cx| {
            this.trigger_completion(window, cx);
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &PreviousForgeCompletion, window, cx| {
            this.navigate_completion(-1, window, cx);
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &NextForgeCompletion, window, cx| {
            this.navigate_completion(1, window, cx);
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &InsertForgeNewline, window, cx| {
            this.insert_newline(window, cx);
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &DeleteForgeWordBackward, window, cx| {
            this.edit_word(WordAction::DeleteBackward, window, cx);
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &DeleteForgeWordForward, window, cx| {
            this.edit_word(WordAction::DeleteForward, window, cx);
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &MoveForgeWordBackward, window, cx| {
            this.edit_word(WordAction::MoveBackward, window, cx);
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &MoveForgeWordForward, window, cx| {
            this.edit_word(WordAction::MoveForward, window, cx);
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &SelectForgeWordBackward, window, cx| {
            this.edit_word(WordAction::SelectBackward, window, cx);
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &SelectForgeWordForward, window, cx| {
            this.edit_word(WordAction::SelectForward, window, cx);
            cx.stop_propagation();
        }))
        .on_action(cx.listener(|this, _: &FindInForgeOutput, window, cx| {
            super::controller::ForgeController::find_in_output(this, window, cx);
            cx.stop_propagation();
        }))
}
