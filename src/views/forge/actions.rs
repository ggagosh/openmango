use gpui::{Context, Div, InteractiveElement, Window};

use crate::keyboard::{
    CancelForgeRun, ClearForgeOutput, FindInForgeOutput, FocusForgeEditor, FocusForgeOutput,
    RunForgeAll, RunForgeSelectionOrStatement,
};

use super::ForgeView;

pub fn bind_root_actions(root: Div, _window: &mut Window, cx: &mut Context<ForgeView>) -> Div {
    root.on_action(cx.listener(|this, _: &RunForgeAll, _window, cx| {
        super::controller::ForgeController::run_all(this, cx);
        cx.stop_propagation();
    }))
    .on_action(cx.listener(|this, _: &RunForgeSelectionOrStatement, window, cx| {
        super::controller::ForgeController::run_selection_or_statement(this, window, cx);
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
