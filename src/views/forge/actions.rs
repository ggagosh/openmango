use gpui_kit::{Context, Div, Entity, InteractiveElement};

use crate::components::request_connection_write;
use crate::state::AppState;

use crate::keyboard::{
    CancelForgeRun, ClearForgeOutput, FindInForgeOutput, FocusForgeEditor, FocusForgeOutput,
    RunForgeAll, RunForgeSelectionOrStatement,
};

use super::ForgeView;

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
    root.on_action(move |_: &RunForgeAll, window, cx| {
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
            move |_window, cx| {
                view.update(cx, |view, cx| {
                    super::controller::ForgeController::run_all(view, cx);
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
    .on_action(cx.listener(|this, _: &FindInForgeOutput, window, cx| {
        super::controller::ForgeController::find_in_output(this, window, cx);
        cx.stop_propagation();
    }))
}
