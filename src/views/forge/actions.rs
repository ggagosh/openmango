use gpui::{Context, Div, InteractiveElement, Window};

use crate::components::request_connection_write;

use crate::keyboard::{
    CancelForgeRun, ClearForgeOutput, FindInForgeOutput, FocusForgeEditor, FocusForgeOutput,
    RunForgeAll, RunForgeSelectionOrStatement,
};

use super::ForgeView;

pub fn bind_root_actions(root: Div, _window: &mut Window, cx: &mut Context<ForgeView>) -> Div {
    root.on_action(cx.listener(|this, _: &RunForgeAll, window, cx| {
        let Some(key) = this.app_state.read(cx).active_forge_tab_key().cloned() else {
            return;
        };
        let app_state = this.app_state.clone();
        let view = cx.entity();
        request_connection_write(
            app_state,
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
    }))
    .on_action(cx.listener(|this, _: &RunForgeSelectionOrStatement, window, cx| {
        let Some(key) = this.app_state.read(cx).active_forge_tab_key().cloned() else {
            return;
        };
        let app_state = this.app_state.clone();
        let view = cx.entity();
        request_connection_write(
            app_state,
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
    }))
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
