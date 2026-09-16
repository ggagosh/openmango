use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::input::Editor;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::state::SessionKey;
use crate::state::app_state::PipelineState;
use crate::theme::spacing;
use crate::views::CollectionView;

use super::stage_editor::panel;
use super::stage_list::pipeline_header_controls;

impl CollectionView {
    pub(in crate::views::documents) fn render_aggregation_text_mode(
        &mut self,
        pipeline: &PipelineState,
        session_key: Option<SessionKey>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let appearance = self.state.read(cx).settings.appearance.clone();
        let disabled_count = pipeline.stages.iter().filter(|stage| !stage.enabled).count();
        let hint = match disabled_count {
            0 => "One array, one object per stage. Comment out a stage to skip it.".to_string(),
            1 => "1 stage is commented out and skipped.".to_string(),
            n => format!("{n} stages are commented out and skipped."),
        };

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(spacing::sm())
            .px(spacing::sm())
            .py(spacing::xs())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .min_w(px(0.0))
                    .child(div().text_sm().child("Pipeline"))
                    .child(
                        div()
                            .min_w(px(0.0))
                            .truncate()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(hint),
                    ),
            )
            .child(pipeline_header_controls(pipeline, session_key.clone(), self.state.clone(), cx));

        panel(&appearance, cx)
            .child(header)
            .child(div().flex().flex_1().min_h(px(0.0)).when_some(
                self.aggregation_text_state.clone(),
                |slot, text_state| {
                    slot.child(
                        Editor::new(&text_state)
                            .font_family(crate::theme::fonts::mono())
                            .aria_label("Pipeline text")
                            .w_full()
                            .h_full()
                            .disabled(session_key.is_none()),
                    )
                },
            ))
            .when_some(self.aggregation_text_error.clone(), |panel, error| {
                panel.child(
                    div().p(spacing::xs()).child(
                        Alert::error("agg-text-error", error)
                            .title("The stages still show the last valid version"),
                    ),
                )
            })
            .into_any_element()
    }
}
