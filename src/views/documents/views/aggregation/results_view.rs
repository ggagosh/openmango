use gpui::*;
use gpui_component::Sizable as _;
use gpui_component::input::Input;
use gpui_component::scroll::ScrollableElement;
use gpui_component::spinner::Spinner;
use gpui_component::tree::tree;

use crate::bson::DocumentKey;
use crate::state::app_state::PipelineState;
use crate::state::{SessionDocument, SessionKey};
use crate::theme::{colors, spacing};
use crate::views::documents::tree::document_tree::build_documents_tree;
use crate::views::documents::tree::tree_row::render_readonly_tree_row;

use crate::views::CollectionView;

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

impl CollectionView {
    pub(in crate::views::documents) fn render_aggregation_results(
        &mut self,
        pipeline: &PipelineState,
        session_key: Option<SessionKey>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let results_count = pipeline.results.as_ref().map(|docs| docs.len()).unwrap_or(0);
        let stage_label = pipeline
            .selected_stage
            .and_then(|idx| pipeline.stages.get(idx).map(|stage| (idx, stage)))
            .map(|(idx, stage)| format!("Results (after Stage {}: {})", idx + 1, stage.operator))
            .unwrap_or_else(|| "Results (full pipeline)".to_string());

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .px(spacing::sm())
            .py(spacing::xs())
            .bg(colors::bg_header())
            .border_b_1()
            .border_color(colors::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::xs())
                    .child(div().text_sm().text_color(colors::text_primary()).child(stage_label))
                    .child(
                        div()
                            .text_xs()
                            .text_color(colors::text_muted())
                            .child(format!("{results_count} document(s)")),
                    )
                    .child(if pipeline.loading {
                        Spinner::new().xsmall().into_any_element()
                    } else {
                        div().into_any_element()
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::xs())
                    .child(div().text_xs().text_color(colors::text_muted()).child("Limit"))
                    .child(if let Some(limit_state) = self.aggregation_limit_state.clone() {
                        Input::new(&limit_state)
                            .w(px(72.0))
                            .disabled(session_key.is_none())
                            .into_any_element()
                    } else {
                        div().into_any_element()
                    }),
            );

        let mut body =
            div().flex().flex_col().flex_1().min_w(px(0.0)).min_h(px(0.0)).overflow_hidden();
        if let Some(error) = pipeline.error.clone() {
            body = body.child(
                div()
                    .px(spacing::sm())
                    .py(spacing::xs())
                    .text_sm()
                    .text_color(colors::text_error())
                    .child(error),
            );
        }
        body = body.child(render_results_tree(self, pipeline, session_key.clone(), window, cx));

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .bg(colors::bg_app())
            .border_t_1()
            .border_color(colors::border())
            .child(header)
            .child(body)
            .into_any_element()
    }
}

fn render_results_tree(
    view: &mut CollectionView,
    pipeline: &PipelineState,
    _session_key: Option<SessionKey>,
    _window: &mut Window,
    cx: &mut Context<CollectionView>,
) -> AnyElement {
    let Some(tree_state) = view.aggregation_results_tree_state.clone() else {
        return div().into_any_element();
    };
    let view_entity = cx.entity();

    if pipeline.loading {
        return div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap(spacing::sm())
            .child(Spinner::new().small())
            .child(div().text_sm().text_color(colors::text_muted()).child("Running pipeline..."))
            .into_any_element();
    }

    let Some(results) = pipeline.results.as_ref() else {
        return div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_sm()
                    .text_color(colors::text_muted())
                    .child("Run the pipeline to see results"),
            )
            .into_any_element();
    };

    if results.is_empty() {
        return div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .child(div().text_sm().text_color(colors::text_muted()).child("No documents returned"))
            .into_any_element();
    }

    let documents: Vec<SessionDocument> = results
        .iter()
        .enumerate()
        .map(|(idx, doc)| SessionDocument {
            key: DocumentKey::from_document(doc, idx),
            doc: doc.clone(),
        })
        .collect();

    let signature = results_signature(&documents);
    if view.aggregation_results_signature != Some(signature) {
        view.aggregation_results_signature = Some(signature);
        view.aggregation_results_expanded_nodes.clear();
    }

    let expanded_nodes = &view.aggregation_results_expanded_nodes;
    let (items, meta, _) = build_documents_tree(&documents, &HashMap::new(), expanded_nodes);
    let node_meta = Arc::new(meta);

    tree_state.update(cx, |tree, cx| {
        tree.set_items(items, cx);
        tree.set_selected_index(None, cx);
    });

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .child(
            div()
                .flex()
                .items_center()
                .px(spacing::lg())
                .py(spacing::xs())
                .bg(colors::bg_header())
                .border_b_1()
                .border_color(colors::border())
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_xs()
                        .text_color(colors::text_muted())
                        .child("Key"),
                )
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_xs()
                        .text_color(colors::text_muted())
                        .child("Value"),
                )
                .child(div().w(px(120.0)).text_xs().text_color(colors::text_muted()).child("Type")),
        )
        .child(div().flex().flex_1().min_w(px(0.0)).overflow_y_scrollbar().child(tree(
            &tree_state,
            {
                let node_meta = node_meta.clone();
                let view_entity = view_entity.clone();
                let tree_state = tree_state.clone();
                move |ix, entry, selected, _window, _cx| {
                    render_readonly_tree_row(
                        ix,
                        entry,
                        selected,
                        &node_meta,
                        view_entity.clone(),
                        tree_state.clone(),
                    )
                }
            },
        )))
        .into_any_element()
}

fn results_signature(documents: &[SessionDocument]) -> u64 {
    let mut hasher = DefaultHasher::new();
    documents.len().hash(&mut hasher);
    for doc in documents {
        doc.key.hash(&mut hasher);
    }
    hasher.finish()
}
