//! What is known about this collection's relations, said beside its name.
//!
//! Three things a person in a collection cannot otherwise tell: that nobody has looked for
//! relations in this database yet, that a search is running, or that there are relations here
//! and some arrived since the canvas was last open. One chip says whichever is true, and a
//! click does the obvious next thing: start the search, or open the canvas on this collection.

use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::state::relations::export::believed;
use crate::state::relations::{Origin, RelationGraph};
use crate::state::{AppCommands, AppState, SessionKey};
use crate::theme::{borders, spacing};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Chip {
    /// Nobody has read this database for relations. Offered once, until someone does.
    NeverInferred,
    Inferring {
        done: usize,
        total: usize,
    },
    /// Relations that touch this collection, and how many in the database are new.
    Known {
        touching: usize,
        unseen: usize,
    },
}

/// What to say, or nothing: a collection with no relations in a database that has been read is
/// not news, and a chip saying so on every such collection would be noise.
fn chip(
    graph: &RelationGraph,
    running: Option<(usize, usize)>,
    unseen: usize,
    database: &str,
    collection: &str,
) -> Option<Chip> {
    if let Some((done, total)) = running {
        return Some(Chip::Inferring { done, total });
    }
    // Files written before "last read" was recorded have no date, but a relation that was
    // sampled proves a search ran.
    let read_before = graph.inferred_at(database).is_some()
        || graph.relations().iter().any(|relation| {
            relation.source.database == database && relation.origin == Origin::Inferred
        });
    if !read_before {
        return Some(Chip::NeverInferred);
    }
    let touching = believed(graph, database)
        .iter()
        .filter(|relation| {
            relation.source.collection == collection || relation.target.collection == collection
        })
        .count();
    (touching > 0 || unseen > 0).then_some(Chip::Known { touching, unseen })
}

pub fn render_relations_chip(
    state: &Entity<AppState>,
    session: Option<&SessionKey>,
    cx: &App,
) -> Option<AnyElement> {
    let session = session?;
    let state_ref = state.read(cx);
    let running = state_ref
        .inference_run()
        .filter(|run| run.database == session.database)
        .map(|run| (run.done + 1, run.total));
    let chip = chip(
        state_ref.relations(),
        running,
        state_ref.unseen_relations(&session.database),
        &session.database,
        &session.collection,
    )?;

    let label = match &chip {
        Chip::NeverInferred => "Infer relations".to_string(),
        Chip::Inferring { done, total } => format!("Reading relations, {done} of {total}"),
        Chip::Known { touching, unseen: 0 } => match touching {
            1 => "1 relation".to_string(),
            count => format!("{count} relations"),
        },
        Chip::Known { touching, unseen } => format!("{touching} relations · {unseen} new"),
    };
    let tooltip = match &chip {
        Chip::NeverInferred => {
            "Nobody has looked for relations in this database yet. Reads a sample of every \
             collection and confirms each guess against the data."
        }
        Chip::Inferring { .. } => "Looking for relations across the database",
        Chip::Known { .. } => "Open the relation canvas on this collection",
    };
    let is_new = matches!(chip, Chip::Known { unseen, .. } if unseen > 0);
    let busy = matches!(chip, Chip::Inferring { .. });

    let state = state.clone();
    let (database, collection) = (session.database.clone(), session.collection.clone());
    Some(
        div()
            .id("collection-relations-chip")
            .flex()
            .items_center()
            .gap(spacing::xs())
            .px(spacing::sm())
            .py(px(1.0))
            .rounded(borders::radius_sm())
            .border_1()
            .border_color(cx.theme().border)
            .text_xs()
            // New relations are the one state worth a second look, so only it takes the accent.
            .text_color(if is_new { cx.theme().primary } else { cx.theme().muted_foreground })
            .child(if busy {
                Spinner::new().xsmall().into_any_element()
            } else {
                Icon::new(crate::assets::AppIcon::Workflow).xsmall().into_any_element()
            })
            .child(label)
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(tooltip).build(window, cx)
            })
            .when(!busy, |chip_el| {
                chip_el
                    .cursor_pointer()
                    .hover(|style| {
                        style.bg(cx.theme().list_hover).text_color(cx.theme().foreground)
                    })
                    .on_click(move |_, _window, cx| match &chip {
                        Chip::NeverInferred => AppCommands::infer_relations_for_database(
                            state.clone(),
                            database.clone(),
                            cx,
                        ),
                        _ => state.update(cx, |state, cx| {
                            state.request_relations_focus(collection.clone());
                            state.open_relations_tab(database.clone(), cx);
                        }),
                    })
            })
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::{Chip, chip};
    use crate::state::relations::{Evidence, FieldRef, Origin, Relation, RelationGraph};

    fn followed(graph: &mut RelationGraph, collection: &str, path: &str, target: &str) {
        graph.upsert(Relation::asserted(
            FieldRef::new("shop", collection, path),
            FieldRef::id_of("shop", target),
            Origin::Probe,
        ));
    }

    #[test]
    fn a_database_nobody_has_read_offers_to_be_read() {
        let mut graph = RelationGraph::new();
        assert_eq!(chip(&graph, None, 0, "shop", "orders"), Some(Chip::NeverInferred));

        // Following a link teaches one relation; it is not a search of the database.
        followed(&mut graph, "orders", "userId", "users");
        assert_eq!(chip(&graph, None, 0, "shop", "orders"), Some(Chip::NeverInferred));
    }

    #[test]
    fn once_read_it_counts_what_touches_this_collection() {
        let mut graph = RelationGraph::new();
        followed(&mut graph, "orders", "userId", "users");
        followed(&mut graph, "reviews", "orderId", "orders");
        followed(&mut graph, "users", "companyId", "companies");
        graph.mark_inferred("shop", Utc::now());

        assert_eq!(
            chip(&graph, None, 0, "shop", "orders"),
            Some(Chip::Known { touching: 2, unseen: 0 })
        );
        assert_eq!(
            chip(&graph, None, 3, "shop", "orders"),
            Some(Chip::Known { touching: 2, unseen: 3 })
        );
        // Read, and nothing here: not worth a chip.
        assert_eq!(chip(&graph, None, 0, "shop", "carts"), None);
        // But new relations elsewhere in the database are, wherever you happen to be.
        assert_eq!(
            chip(&graph, None, 3, "shop", "carts"),
            Some(Chip::Known { touching: 0, unseen: 3 })
        );
    }

    #[test]
    fn a_running_search_is_what_it_says_whatever_else_is_true() {
        let graph = RelationGraph::new();
        assert_eq!(
            chip(&graph, Some((4, 58)), 0, "shop", "orders"),
            Some(Chip::Inferring { done: 4, total: 58 })
        );
    }

    #[test]
    fn a_file_from_before_reads_were_dated_still_counts_as_read() {
        let mut graph = RelationGraph::new();
        graph.upsert(Relation::candidate(
            FieldRef::new("shop", "orders", "userId"),
            FieldRef::id_of("shop", "users"),
            1.0,
            Evidence { probed: 20, hits: 20, sampled: 400, sampled_at: Utc::now() },
        ));

        assert_eq!(graph.inferred_at("shop"), None);
        assert_eq!(
            chip(&graph, None, 0, "shop", "orders"),
            Some(Chip::Known { touching: 1, unseen: 0 })
        );
    }
}
