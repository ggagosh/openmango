//! Where each collection sits on the relation canvas.
//!
//! Layered, left to right: a collection sits to the left of what it points at, so the
//! collections everything leans on gather on the right and a reference reads the way it is
//! written. Coordinates are in world units, which the canvas scales by its zoom; nothing here
//! knows about pixels, the window or the theme, which is what lets it be tested and cached.
//!
//! ponytail: long edges are not routed around the cards they pass, they run beneath them. The
//! fix is dummy nodes in the layers an edge crosses, worth it if hovering stops being enough to
//! read a busy database.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};

use super::{FieldRef, RelationGraph, Status};

pub const CARD_WIDTH: f32 = 230.0;
pub const HEADER_HEIGHT: f32 = 30.0;
pub const FIELD_HEIGHT: f32 = 20.0;
/// Room between columns for the curves to turn in.
const COLUMN_GAP: f32 = 150.0;
const ROW_GAP: f32 = 22.0;
/// A layer taller than this is split into columns side by side. Most collections in a real
/// database point straight at one or two hubs, so without the split they form a single column
/// thousands of units tall and fitting it to the window makes every card unreadable.
const MAX_COLUMN_HEIGHT: f32 = 1500.0;
const ORDERING_SWEEPS: usize = 4;

/// One reference field listed on a card.
#[derive(Debug, Clone, PartialEq)]
pub struct CanvasField {
    pub path: String,
    /// Points back into its own collection. Drawn as a mark on the row, not as an edge: a loop
    /// says nothing the mark does not.
    pub to_self: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CanvasNode {
    pub collection: String,
    pub fields: Vec<CanvasField>,
    /// How many fields elsewhere point here.
    pub incoming: usize,
    pub x: f32,
    pub y: f32,
    pub height: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CanvasEdge {
    pub source: usize,
    /// Index into the source node's `fields`.
    pub field: usize,
    pub target: usize,
    /// Reviewed or asserted, as opposed to a guess.
    pub accepted: bool,
    pub from: FieldRef,
    pub to: FieldRef,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CanvasLayout {
    pub nodes: Vec<CanvasNode>,
    pub edges: Vec<CanvasEdge>,
    pub width: f32,
    pub height: f32,
}

/// The two ends of an edge and the direction its curve leaves in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeLine {
    pub start: (f32, f32),
    pub end: (f32, f32),
    /// True when the edge runs left to right. A reference into an earlier column leaves from
    /// the card's left side instead, so the curve never doubles back through its own card.
    pub rightwards: bool,
}

impl CanvasLayout {
    pub fn index_of(&self, collection: &str) -> Option<usize> {
        self.nodes.iter().position(|node| node.collection == collection)
    }

    /// From the field's row on the source card to the target card's header.
    pub fn edge_line(&self, edge: &CanvasEdge) -> EdgeLine {
        let source = &self.nodes[edge.source];
        let target = &self.nodes[edge.target];
        let rightwards = target.x >= source.x;
        let row = source.y + HEADER_HEIGHT + (edge.field as f32 + 0.5) * FIELD_HEIGHT;
        let header = target.y + HEADER_HEIGHT / 2.0;
        if rightwards {
            EdgeLine { start: (source.x + CARD_WIDTH, row), end: (target.x, header), rightwards }
        } else {
            EdgeLine { start: (source.x, row), end: (target.x + CARD_WIDTH, header), rightwards }
        }
    }

    /// Every node joined to `node` by an edge, in either direction.
    pub fn neighbours(&self, node: usize) -> BTreeSet<usize> {
        self.edges
            .iter()
            .filter_map(|edge| match (edge.source == node, edge.target == node) {
                (true, _) => Some(edge.target),
                (_, true) => Some(edge.source),
                _ => None,
            })
            .collect()
    }
}

/// Changes whenever the layout would: a relation arriving, leaving, or being reviewed. Cheap
/// enough to compute every frame, which is what lets the layout itself be computed only when
/// this moves.
pub fn fingerprint(graph: &RelationGraph, database: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for relation in graph.relations() {
        if relation.source.database == database {
            relation.source.hash(&mut hasher);
            relation.target.hash(&mut hasher);
            relation.status.hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Lay out every collection of `database` that a relation touches. Rejected relations are left
/// out, and so are targets in another database, which have no card to arrive at.
pub fn layout(graph: &RelationGraph, database: &str) -> CanvasLayout {
    let relations: Vec<_> = graph
        .relations()
        .iter()
        .filter(|relation| {
            relation.status != Status::Rejected
                && relation.source.database == database
                && relation.target.database == database
        })
        .collect();

    // BTree collections throughout: the same graph must always produce the same picture.
    let mut fields: BTreeMap<&str, BTreeMap<&str, bool>> = BTreeMap::new();
    for relation in &relations {
        let to_self = relation.source.collection == relation.target.collection;
        let field = fields
            .entry(relation.source.collection.as_str())
            .or_default()
            .entry(relation.source.path.as_str())
            .or_default();
        *field |= to_self;
        fields.entry(relation.target.collection.as_str()).or_default();
    }

    let mut nodes: Vec<CanvasNode> = fields
        .iter()
        .map(|(collection, fields)| CanvasNode {
            collection: collection.to_string(),
            fields: fields
                .iter()
                .map(|(path, to_self)| CanvasField { path: path.to_string(), to_self: *to_self })
                .collect(),
            incoming: 0,
            x: 0.0,
            y: 0.0,
            height: HEADER_HEIGHT + fields.len() as f32 * FIELD_HEIGHT,
        })
        .collect();
    let index: HashMap<&str, usize> =
        fields.keys().enumerate().map(|(index, collection)| (*collection, index)).collect();

    let mut edges = Vec::new();
    for relation in &relations {
        let source = index[relation.source.collection.as_str()];
        let target = index[relation.target.collection.as_str()];
        if source == target {
            continue;
        }
        nodes[target].incoming += 1;
        let field = nodes[source]
            .fields
            .iter()
            .position(|field| field.path == relation.source.path)
            .unwrap_or_default();
        edges.push(CanvasEdge {
            source,
            field,
            target,
            accepted: relation.status == Status::Accepted,
            from: relation.source.clone(),
            to: relation.target.clone(),
        });
    }
    edges.sort_by_key(|edge| (edge.source, edge.field, edge.target));

    let links: BTreeSet<(usize, usize)> =
        edges.iter().map(|edge| (edge.source, edge.target)).collect();
    let layers = assign_layers(nodes.len(), &links);
    let columns = order_columns(&nodes, &layers, &links);
    let (width, height) = place(&mut nodes, &columns);

    CanvasLayout { nodes, edges, width, height }
}

/// The layer of each node: every link runs from a lower layer to a higher one.
fn assign_layers(count: usize, links: &BTreeSet<(usize, usize)>) -> Vec<usize> {
    let forward = break_cycles(count, links);
    let mut successors = vec![Vec::new(); count];
    let mut pending = vec![0usize; count];
    for &(from, to) in &forward {
        successors[from].push(to);
        pending[to] += 1;
    }

    // Longest path from the left, in topological order.
    let mut layers = vec![0usize; count];
    let mut ready: Vec<usize> = (0..count).filter(|&node| pending[node] == 0).collect();
    let mut order = Vec::with_capacity(count);
    while let Some(node) = ready.pop() {
        order.push(node);
        for &next in &successors[node] {
            layers[next] = layers[next].max(layers[node] + 1);
            pending[next] -= 1;
            if pending[next] == 0 {
                ready.push(next);
            }
        }
    }

    // Then pull everything as far right as its targets allow. Longest-path alone strands every
    // leaf in the first column, however far away the one thing it points at ended up.
    for &node in order.iter().rev() {
        if let Some(nearest) = successors[node].iter().map(|&next| layers[next]).min() {
            layers[node] = nearest - 1;
        }
    }
    layers
}

/// The links with every cycle broken by turning one of its edges around, for layering only.
/// Two collections that point at each other still have to sit in some order.
fn break_cycles(count: usize, links: &BTreeSet<(usize, usize)>) -> BTreeSet<(usize, usize)> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Unseen,
        Open,
        Done,
    }

    let mut successors = vec![Vec::new(); count];
    for &(from, to) in links {
        successors[from].push(to);
    }
    let mut marks = vec![Mark::Unseen; count];
    let mut forward = BTreeSet::new();

    for root in 0..count {
        if marks[root] != Mark::Unseen {
            continue;
        }
        // Iterative, so a long chain of collections cannot overflow the stack.
        let mut stack = vec![(root, 0usize)];
        marks[root] = Mark::Open;
        while let Some(&(node, next)) = stack.last() {
            let Some(&to) = successors[node].get(next) else {
                marks[node] = Mark::Done;
                stack.pop();
                continue;
            };
            stack.last_mut().expect("just read").1 += 1;
            match marks[to] {
                // Back onto the path being walked: this is the edge that closes a cycle.
                Mark::Open => {
                    forward.insert((to, node));
                }
                Mark::Done => {
                    forward.insert((node, to));
                }
                Mark::Unseen => {
                    forward.insert((node, to));
                    marks[to] = Mark::Open;
                    stack.push((to, 0));
                }
            }
        }
    }
    forward
}

/// Nodes grouped into columns, left to right, each ordered top to bottom so that a node sits
/// near what it is joined to. A layer too tall for one column becomes several.
fn order_columns(
    nodes: &[CanvasNode],
    layers: &[usize],
    links: &BTreeSet<(usize, usize)>,
) -> Vec<Vec<usize>> {
    let layer_count = layers.iter().max().map_or(0, |max| max + 1);
    let mut by_layer: Vec<Vec<usize>> = vec![Vec::new(); layer_count];
    for (node, &layer) in layers.iter().enumerate() {
        by_layer[layer].push(node);
    }

    let mut joined = vec![Vec::new(); nodes.len()];
    for &(from, to) in links {
        joined[from].push(to);
        joined[to].push(from);
    }

    // Barycentre sweeps: each node moves to the average position of what it is joined to in
    // the layers beside it. A few passes settle it; more do not visibly help.
    let mut position = vec![0.0f32; nodes.len()];
    for layer in &by_layer {
        for (slot, &node) in layer.iter().enumerate() {
            position[node] = slot as f32;
        }
    }
    for sweep in 0..ORDERING_SWEEPS {
        let order: Vec<usize> = if sweep % 2 == 0 {
            (0..layer_count).collect()
        } else {
            (0..layer_count).rev().collect()
        };
        for layer in order {
            let mut ranked: Vec<(f32, usize)> = by_layer[layer]
                .iter()
                .map(|&node| {
                    let beside: Vec<f32> = joined[node]
                        .iter()
                        .filter(|&&other| layers[other] != layer)
                        .map(|&other| position[other])
                        .collect();
                    let centre = if beside.is_empty() {
                        position[node]
                    } else {
                        beside.iter().sum::<f32>() / beside.len() as f32
                    };
                    (centre, node)
                })
                .collect();
            ranked.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            by_layer[layer] = ranked.iter().map(|&(_, node)| node).collect();
            for (slot, &node) in by_layer[layer].iter().enumerate() {
                position[node] = slot as f32;
            }
        }
    }

    by_layer.into_iter().flat_map(|layer| split_tall(nodes, layer)).collect()
}

/// One layer as one or more columns of roughly equal height.
fn split_tall(nodes: &[CanvasNode], layer: Vec<usize>) -> Vec<Vec<usize>> {
    let total: f32 = layer.iter().map(|&node| nodes[node].height + ROW_GAP).sum();
    let wanted = (total / MAX_COLUMN_HEIGHT).ceil().max(1.0);
    let budget = total / wanted;

    let mut columns = vec![Vec::new()];
    let mut filled = 0.0;
    for node in layer {
        let height = nodes[node].height + ROW_GAP;
        if filled > 0.0 && filled + height / 2.0 > budget && (columns.len() as f32) < wanted {
            columns.push(Vec::new());
            filled = 0.0;
        }
        columns.last_mut().expect("starts with one").push(node);
        filled += height;
    }
    columns
}

/// Give every node its coordinates. Columns are centred on a shared midline, so a short column
/// sits beside the middle of a tall one rather than hanging from its top.
fn place(nodes: &mut [CanvasNode], columns: &[Vec<usize>]) -> (f32, f32) {
    let column_height = |column: &Vec<usize>| -> f32 {
        let cards: f32 = column.iter().map(|&node| nodes[node].height).sum();
        cards + ROW_GAP * column.len().saturating_sub(1) as f32
    };
    let heights: Vec<f32> = columns.iter().map(column_height).collect();
    let tallest = heights.iter().copied().fold(0.0, f32::max);

    for (slot, column) in columns.iter().enumerate() {
        let mut y = (tallest - heights[slot]) / 2.0;
        for &node in column {
            nodes[node].x = slot as f32 * (CARD_WIDTH + COLUMN_GAP);
            nodes[node].y = y;
            y += nodes[node].height + ROW_GAP;
        }
    }

    let width = match columns.len() {
        0 => 0.0,
        count => count as f32 * CARD_WIDTH + (count - 1) as f32 * COLUMN_GAP,
    };
    (width, tallest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::relations::{Origin, Relation};

    fn graph(links: &[(&str, &str, &str)]) -> RelationGraph {
        let mut graph = RelationGraph::new();
        for (collection, path, target) in links {
            graph.upsert(Relation::asserted(
                FieldRef::new("shop", *collection, *path),
                FieldRef::id_of("shop", *target),
                Origin::Probe,
            ));
        }
        graph
    }

    fn x_of(layout: &CanvasLayout, collection: &str) -> f32 {
        layout.nodes[layout.index_of(collection).expect("laid out")].x
    }

    #[test]
    fn a_collection_sits_left_of_what_it_points_at() {
        let layout = layout(
            &graph(&[("orders", "userId", "users"), ("users", "companyId", "companies")]),
            "shop",
        );

        assert!(x_of(&layout, "orders") < x_of(&layout, "users"));
        assert!(x_of(&layout, "users") < x_of(&layout, "companies"));
        assert_eq!(layout.edges.len(), 2);
    }

    #[test]
    fn a_leaf_sits_beside_its_target_not_stranded_in_the_first_column() {
        // `logs` points only at `companies`, two layers from the left.
        let layout = layout(
            &graph(&[
                ("orders", "userId", "users"),
                ("users", "companyId", "companies"),
                ("logs", "companyId", "companies"),
            ]),
            "shop",
        );

        assert_eq!(x_of(&layout, "logs"), x_of(&layout, "users"));
    }

    #[test]
    fn collections_that_point_at_each_other_are_still_placed() {
        let layout =
            layout(&graph(&[("users", "teamId", "teams"), ("teams", "ownerId", "users")]), "shop");

        assert_eq!(layout.nodes.len(), 2);
        assert_eq!(layout.edges.len(), 2);
        assert_ne!(x_of(&layout, "users"), x_of(&layout, "teams"));
        // One of the two runs against the grain and says so, so its curve leaves leftwards.
        let against = layout.edges.iter().filter(|e| !layout.edge_line(e).rightwards).count();
        assert_eq!(against, 1);
    }

    #[test]
    fn a_self_reference_marks_its_field_and_draws_no_edge() {
        let layout = layout(&graph(&[("categories", "parentId", "categories")]), "shop");

        assert_eq!(layout.nodes.len(), 1);
        assert!(layout.edges.is_empty());
        assert!(layout.nodes[0].fields[0].to_self);
        assert_eq!(layout.nodes[0].incoming, 0);
    }

    #[test]
    fn a_hub_with_many_sources_wraps_them_into_several_columns() {
        let names: Vec<String> = (0..80).map(|n| format!("source{n:02}")).collect();
        let links: Vec<(&str, &str, &str)> =
            names.iter().map(|name| (name.as_str(), "userId", "users")).collect();
        let layout = layout(&graph(&links), "shop");

        let columns: BTreeSet<i32> = layout.nodes.iter().map(|node| node.x as i32).collect();
        assert!(columns.len() > 2, "eighty sources in one column is unreadable");
        assert!(layout.height <= MAX_COLUMN_HEIGHT + HEADER_HEIGHT + FIELD_HEIGHT + ROW_GAP);
        assert_eq!(layout.nodes[layout.index_of("users").unwrap()].incoming, 80);
    }

    #[test]
    fn cards_in_a_column_never_overlap() {
        let layout = layout(
            &graph(&[
                ("orders", "userId", "users"),
                ("orders", "items[].productId", "products"),
                ("invoices", "userId", "users"),
                ("reviews", "productId", "products"),
            ]),
            "shop",
        );

        for a in &layout.nodes {
            for b in &layout.nodes {
                if a.collection != b.collection && a.x == b.x {
                    assert!(a.y + a.height <= b.y || b.y + b.height <= a.y);
                }
            }
        }
    }

    #[test]
    fn an_edge_runs_from_its_field_row_to_the_target_header() {
        let layout = layout(
            &graph(&[("orders", "items[].productId", "products"), ("orders", "userId", "users")]),
            "shop",
        );
        let orders = &layout.nodes[layout.index_of("orders").unwrap()];
        let edge = layout.edges.iter().find(|edge| edge.from.path == "userId").unwrap();
        let line = layout.edge_line(edge);

        // `userId` sorts after `items[].productId`, so it is the second row.
        assert_eq!(
            line.start,
            (orders.x + CARD_WIDTH, orders.y + HEADER_HEIGHT + 1.5 * FIELD_HEIGHT)
        );
        assert!(line.rightwards);
    }

    #[test]
    fn rejected_relations_and_other_databases_are_left_out() {
        let mut graph = graph(&[("orders", "userId", "users"), ("orders", "couponId", "coupons")]);
        graph.set_status(
            &FieldRef::new("shop", "orders", "couponId"),
            &FieldRef::id_of("shop", "coupons"),
            Status::Rejected,
        );
        graph.upsert(Relation::asserted(
            FieldRef::new("shop", "orders", "tenantId"),
            FieldRef::id_of("admin", "tenants"),
            Origin::DbRef,
        ));

        let layout = layout(&graph, "shop");

        assert!(layout.index_of("coupons").is_none());
        assert!(layout.index_of("tenants").is_none());
        assert_eq!(layout.edges.len(), 1);
    }

    #[test]
    fn the_same_graph_always_draws_the_same_picture() {
        let links = [
            ("orders", "userId", "users"),
            ("invoices", "orderId", "orders"),
            ("users", "companyId", "companies"),
            ("reviews", "userId", "users"),
        ];
        assert_eq!(layout(&graph(&links), "shop"), layout(&graph(&links), "shop"));
    }

    #[test]
    fn the_fingerprint_moves_when_a_relation_is_reviewed() {
        let mut graph = graph(&[("orders", "userId", "users")]);
        let before = fingerprint(&graph, "shop");
        assert_eq!(before, fingerprint(&graph, "shop"));

        graph.set_status(
            &FieldRef::new("shop", "orders", "userId"),
            &FieldRef::id_of("shop", "users"),
            Status::Rejected,
        );
        assert_ne!(before, fingerprint(&graph, "shop"));
        assert_eq!(
            fingerprint(&graph, "elsewhere"),
            fingerprint(&RelationGraph::new(), "elsewhere")
        );
    }
}
