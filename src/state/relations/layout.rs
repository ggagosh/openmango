//! Where each collection sits on the relation canvas, and the path each relation takes.
//!
//! The method is the standard one for drawing a directed graph in layers, due to Sugiyama and
//! used by Graphviz `dot`, dagre and ELK: rank the nodes so edges run one way (network simplex,
//! which minimises total edge length), order each rank to minimise crossings, place nodes beside
//! what they are joined to (Brandes–Köpf), and give every long edge a lane of its own through
//! the ranks it crosses, so it runs between cards rather than beneath them. `dugong` is a port
//! of dagre and does that part; what is here is what an ER drawing needs on top of it:
//!
//! - one routed trunk per pair of collections, however many fields join them, with each field's
//!   edge leaving its own row and merging into the trunk;
//! - a rank too tall to read folded into columns whose edges share a bus, which real databases
//!   need because most of their collections point straight at one or two hubs;
//! - smooth paths through the routed points, as cubic segments ready to draw.
//!
//! Coordinates are world units, which the canvas scales by its zoom. Nothing here knows about
//! pixels, the window or the theme, which is what lets it be tested and cached.
//!
//! ponytail: this runs where it is called, on the main thread, a few milliseconds for a real
//! database and only when the graph changes. Move it to a background task if a database ever
//! makes that noticeable.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};

use dugong::graphlib::{Graph, GraphOptions};
use dugong::{EdgeLabel, GraphLabel, NodeLabel, RankDir};

use super::{FieldRef, RelationGraph, Status};

pub const CARD_WIDTH: f32 = 230.0;
pub const HEADER_HEIGHT: f32 = 30.0;
pub const FIELD_HEIGHT: f32 = 20.0;
/// Room between ranks for the curves to turn in.
const RANK_GAP: f64 = 150.0;
const CARD_GAP: f64 = 22.0;
/// Room between two edges' lanes where they pass a rank side by side.
const LANE_GAP: f64 = 10.0;
/// A rank whose cards stack taller than this is folded into columns. Without it a hub with sixty
/// sources is one column thousands of units tall, and fitting that to a window makes every card
/// unreadable.
const MAX_COLUMN_HEIGHT: f64 = 1500.0;
/// How far an edge travels straight out of a row, and into a header, before it turns. It is what
/// makes an edge read as belonging to its row rather than to the card's corner.
const STUB: f32 = 26.0;
/// The arrowhead's length. The path stops this short of the card so the head stays sharp.
pub const ARROW: f32 = 7.0;

type P = (f32, f32);

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

/// A smooth path from a field's row to a collection's header.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EdgePath {
    pub start: P,
    /// Cubic segments, each two control points and then where the segment ends.
    pub segments: Vec<[P; 3]>,
    /// Where the arrowhead's point lands: on the target card's edge, [`ARROW`] past the path.
    pub tip: P,
    /// The head points right. False for a reference that runs against the grain, which enters
    /// its target from the far side.
    pub rightwards: bool,
    /// Everything the path touches, control points included: `(left, top, right, bottom)`.
    pub bounds: (f32, f32, f32, f32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CanvasEdge {
    pub source: usize,
    /// Index into the source node's `fields`.
    pub field: usize,
    pub target: usize,
    pub from: FieldRef,
    pub to: FieldRef,
    pub path: EdgePath,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CanvasLayout {
    pub nodes: Vec<CanvasNode>,
    pub edges: Vec<CanvasEdge>,
    pub width: f32,
    pub height: f32,
}

impl CanvasLayout {
    pub fn index_of(&self, collection: &str) -> Option<usize> {
        self.nodes.iter().position(|node| node.collection == collection)
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
            from: relation.source.clone(),
            to: relation.target.clone(),
            path: EdgePath::default(),
        });
    }
    edges.sort_by_key(|edge| (edge.source, edge.field, edge.target));

    // One link per pair of collections, weighted by how many fields join them: the heavier the
    // link, the harder the layout works to keep it short and straight.
    let mut links: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for edge in &edges {
        *links.entry((edge.source, edge.target)).or_default() += 1;
    }

    let routes = match place(&mut nodes, &links) {
        Some(routes) => routes,
        None => {
            // The layout engine refused the graph. A plain grid still shows what is known.
            log::warn!("relation layout failed; falling back to a grid");
            place_in_grid(&mut nodes);
            BTreeMap::new()
        }
    };
    for edge in &mut edges {
        let between = routes.get(&(edge.source, edge.target)).map(Vec::as_slice).unwrap_or(&[]);
        edge.path = path_for(&nodes[edge.source], edge.field, &nodes[edge.target], between);
    }

    normalise(nodes, edges)
}

/// What lies between the two cards of each link: the points its trunk was routed through.
type Routes = BTreeMap<(usize, usize), Vec<P>>;

/// Something the layout engine places: a collection's card, or a junction where edges that
/// share a bus join it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Stop {
    Card(usize),
    Junction(usize),
}

impl Stop {
    fn id(self) -> String {
        match self {
            Stop::Card(index) => index.to_string(),
            Stop::Junction(index) => format!("j{index}"),
        }
    }
}

/// The stops each link passes through, ends included. Most go straight from card to card.
type Chains = BTreeMap<(usize, usize), Vec<Stop>>;

/// Place the nodes and route the links between them.
///
/// Laid out twice when a rank comes out too tall to read: once to see the ranks, then again
/// with the overflow folded into columns whose edges share a bus.
fn place(nodes: &mut [CanvasNode], links: &BTreeMap<(usize, usize), usize>) -> Option<Routes> {
    let direct: Chains = links
        .keys()
        .map(|&(source, target)| ((source, target), vec![Stop::Card(source), Stop::Card(target)]))
        .collect();
    let first = run_engine(nodes, links, &direct)?;
    let folded = fold_tall_ranks(nodes, links, &first.0, direct);
    let (centres, routes) = match folded {
        Some(chains) => run_engine(nodes, links, &chains)?,
        None => first,
    };
    for (node, (x, y)) in nodes.iter_mut().zip(centres) {
        node.x = x - CARD_WIDTH / 2.0;
        node.y = y - node.height / 2.0;
    }
    Some(routes)
}

/// Card centres in node order, and each link's route along its chain.
fn run_engine(
    nodes: &[CanvasNode],
    links: &BTreeMap<(usize, usize), usize>,
    chains: &Chains,
) -> Option<(Vec<P>, Routes)> {
    let mut graph: Graph<NodeLabel, EdgeLabel, GraphLabel> =
        Graph::new(GraphOptions { multigraph: false, compound: false, ..Default::default() });
    graph.set_graph(GraphLabel {
        rankdir: RankDir::LR,
        ranksep: RANK_GAP,
        nodesep: CARD_GAP,
        edgesep: LANE_GAP,
        ..Default::default()
    });
    graph.set_default_edge_label(EdgeLabel::default);
    for (index, node) in nodes.iter().enumerate() {
        graph.set_node(
            index.to_string(),
            NodeLabel {
                width: f64::from(CARD_WIDTH),
                height: f64::from(node.height),
                ..Default::default()
            },
        );
    }

    // A hop shared by many links is one heavy edge: the heavier, the straighter it is kept,
    // which is what makes a bus read as a bus.
    let mut hops: BTreeMap<(Stop, Stop), f64> = BTreeMap::new();
    for (link, chain) in chains {
        for pair in chain.windows(2) {
            *hops.entry((pair[0], pair[1])).or_default() += links[link] as f64;
        }
    }
    for stop in hops.keys().flat_map(|&(from, to)| [from, to]) {
        if let Stop::Junction(_) = stop {
            graph.set_node(stop.id(), NodeLabel { width: 1.0, height: 1.0, ..Default::default() });
        }
    }
    for (&(from, to), &weight) in &hops {
        graph.set_edge_with_label(from.id(), to.id(), EdgeLabel { weight, ..Default::default() });
    }

    dugong::layout(&mut graph).ok()?;

    let centre = |stop: Stop| -> Option<P> {
        let label = graph.node(&stop.id())?;
        Some((label.x? as f32, label.y? as f32))
    };
    let mut routes = Routes::new();
    for (&link, chain) in chains {
        let mut route = Vec::new();
        for pair in chain.windows(2) {
            let points = &graph.edge(&pair[0].id(), &pair[1].id(), None)?.points;
            // A hop's own ends are where it met each outline. Between cards those are replaced
            // by a row and a header; at a junction, by the junction itself.
            if points.len() > 2 {
                route.extend(
                    points[1..points.len() - 1]
                        .iter()
                        .map(|point| (point.x as f32, point.y as f32)),
                );
            }
            if let Stop::Junction(_) = pair[1] {
                route.push(centre(pair[1])?);
            }
        }
        routes.insert(link, route);
    }
    let centres = (0..nodes.len()).map(|index| centre(Stop::Card(index))).collect::<Option<_>>()?;
    Some((centres, routes))
}

/// New chains for the links of any rank that stacks too tall, or `None` if every rank is fine.
///
/// A rank is tall when many collections do nothing but point at a hub, or one collection points
/// at many lookups. Those cards are joined on one side only, so they can move a rank outward
/// without dragging the graph with them. Moving them is not enough, though: every long edge is
/// given a straight lane of its own, so the new columns end up staircased and the drawing no
/// shorter. So the edges of a column are bundled: they meet at a junction beside the column and
/// travel on as one line, column to column, to the card they were all going to anyway.
fn fold_tall_ranks(
    nodes: &[CanvasNode],
    links: &BTreeMap<(usize, usize), usize>,
    centres: &[P],
    mut chains: Chains,
) -> Option<Chains> {
    let sources: BTreeSet<usize> = links.keys().map(|&(source, _)| source).collect();
    let targets: BTreeSet<usize> = links.keys().map(|&(_, target)| target).collect();

    // Cards are one width, so the cards of a rank share an x.
    let mut ranks: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for (index, centre) in centres.iter().enumerate() {
        ranks.entry(centre.0.round() as i64).or_default().push(index);
    }

    // One junction per column per far end, found again by every link that shares it.
    let mut junctions: BTreeMap<(i64, usize, usize), usize> = BTreeMap::new();
    let mut folded = false;
    for (&rank, members) in &ranks {
        let stacked = |cards: &[usize]| -> f64 {
            cards.iter().map(|&card| f64::from(nodes[card].height) + CARD_GAP).sum()
        };
        let mut movable: Vec<usize> = members
            .iter()
            .copied()
            .filter(|card| sources.contains(card) != targets.contains(card))
            .collect();
        let total = stacked(members);
        if total <= MAX_COLUMN_HEIGHT || movable.len() < 2 {
            continue;
        }
        // Neighbours in the rank stay neighbours in their column.
        movable.sort_by(|a, b| centres[*a].1.total_cmp(&centres[*b].1));
        let columns = (total / MAX_COLUMN_HEIGHT).ceil();
        let budget = total / columns;
        // Whatever cannot move stays in the first column and counts against its height.
        let (mut column, mut filled) = (0usize, total - stacked(&movable));
        for card in movable {
            let height = f64::from(nodes[card].height) + CARD_GAP;
            if filled + height / 2.0 > budget && (column as f64) < columns - 1.0 {
                column += 1;
                filled = 0.0;
            }
            filled += height;
            if column == 0 {
                continue;
            }
            folded = true;
            let outward = sources.contains(&card);
            for &(source, target) in links.keys().filter(|link| link.0 == card || link.1 == card) {
                let far = if outward { target } else { source };
                let mut bus: Vec<Stop> = (1..=column)
                    .map(|step| {
                        let next = junctions.len();
                        Stop::Junction(*junctions.entry((rank, step, far)).or_insert(next))
                    })
                    .collect();
                // A source's bus runs from its own column in towards the hub; a lookup's runs
                // from the hub out to its column.
                if outward {
                    bus.reverse();
                }
                let mut chain = vec![Stop::Card(source)];
                chain.extend(bus);
                chain.push(Stop::Card(target));
                chains.insert((source, target), chain);
            }
        }
    }
    folded.then_some(chains)
}

/// Rows of cards, left to right. Only reached if the layout engine fails.
fn place_in_grid(nodes: &mut [CanvasNode]) {
    let per_row = (nodes.len() as f32).sqrt().ceil().max(1.0) as usize;
    let tallest = nodes.iter().map(|node| node.height).fold(0.0, f32::max);
    for (index, node) in nodes.iter_mut().enumerate() {
        node.x = (index % per_row) as f32 * (CARD_WIDTH + RANK_GAP as f32);
        node.y = (index / per_row) as f32 * (tallest + CARD_GAP as f32);
    }
}

/// One relation's path: out of its field's row, along the trunk its collections share, and into
/// the target's header.
fn path_for(source: &CanvasNode, field: usize, target: &CanvasNode, between: &[P]) -> EdgePath {
    let source_centre = source.x + CARD_WIDTH / 2.0;
    let target_centre = target.x + CARD_WIDTH / 2.0;

    // Which side each end uses follows the trunk, so a reference that runs against the grain
    // leaves leftwards instead of doubling back through its own card.
    let leaves_right = between.first().map_or(target_centre, |point| point.0) >= source_centre;
    let enters_left = between.last().map_or(source_centre, |point| point.0) <= target_centre;
    let out = if leaves_right { 1.0 } else { -1.0 };
    let into = if enters_left { 1.0 } else { -1.0 };

    let start = (
        if leaves_right { source.x + CARD_WIDTH } else { source.x },
        source.y + HEADER_HEIGHT + (field as f32 + 0.5) * FIELD_HEIGHT,
    );
    let tip = (
        if enters_left { target.x } else { target.x + CARD_WIDTH },
        target.y + HEADER_HEIGHT / 2.0,
    );
    let end = (tip.0 - ARROW * into, tip.1);

    let mut points = vec![start, (start.0 + STUB * out, start.1)];
    points.extend_from_slice(between);
    points.extend([(end.0 - STUB * into, end.1), end]);

    let segments = basis_spline(&points);
    let mut bounds =
        (start.0.min(tip.0), start.1.min(tip.1), start.0.max(tip.0), start.1.max(tip.1));
    for point in segments.iter().flatten() {
        bounds = (
            bounds.0.min(point.0),
            bounds.1.min(point.1),
            bounds.2.max(point.0),
            bounds.3.max(point.1),
        );
    }
    EdgePath { start, segments, tip, rightwards: enters_left, bounds }
}

/// A uniform cubic B-spline through `points`, as Bézier segments. It starts and ends on the
/// first and last point and is pulled towards the ones between without passing through them,
/// which is what turns a route of corners into a line that flows. The same curve d3 calls
/// `curveBasis`, which is what dagre's own renderers draw with.
fn basis_spline(points: &[P]) -> Vec<[P; 3]> {
    let mix = |a: P, b: P, c: P, (wa, wb, wc): (f32, f32, f32)| {
        ((wa * a.0 + wb * b.0 + wc * c.0) / 6.0, (wa * a.1 + wb * b.1 + wc * c.1) / 6.0)
    };
    let line = |from: P, to: P| {
        [mix(from, to, to, (4.0, 2.0, 0.0)), mix(from, to, to, (2.0, 4.0, 0.0)), to]
    };
    let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
        return Vec::new();
    };
    if points.len() < 3 {
        return vec![line(first, last)];
    }

    let mut segments = Vec::with_capacity(points.len() + 1);
    let (mut before, mut at) = (points[0], points[1]);
    let mut pen = mix(before, at, at, (5.0, 1.0, 0.0));
    segments.push(line(first, pen));
    // The last point is fed twice, which is what brings the curve to rest on it.
    for &next in points[2..].iter().chain(std::iter::once(&last)) {
        let to = mix(before, at, next, (1.0, 4.0, 1.0));
        segments.push([
            mix(before, at, at, (4.0, 2.0, 0.0)),
            mix(before, at, at, (2.0, 4.0, 0.0)),
            to,
        ]);
        pen = to;
        (before, at) = (at, next);
    }
    segments.push(line(pen, last));
    segments
}

/// Shift everything so the drawing starts at the origin, and measure it.
fn normalise(mut nodes: Vec<CanvasNode>, mut edges: Vec<CanvasEdge>) -> CanvasLayout {
    let mut bounds = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for node in &nodes {
        bounds = (
            bounds.0.min(node.x),
            bounds.1.min(node.y),
            bounds.2.max(node.x + CARD_WIDTH),
            bounds.3.max(node.y + node.height),
        );
    }
    for edge in &edges {
        let path = edge.path.bounds;
        bounds = (
            bounds.0.min(path.0),
            bounds.1.min(path.1),
            bounds.2.max(path.2),
            bounds.3.max(path.3),
        );
    }
    if nodes.is_empty() {
        return CanvasLayout::default();
    }

    let (dx, dy) = (-bounds.0, -bounds.1);
    let shift = |point: &mut P| *point = (point.0 + dx, point.1 + dy);
    for node in &mut nodes {
        node.x += dx;
        node.y += dy;
    }
    for edge in &mut edges {
        let path = &mut edge.path;
        shift(&mut path.start);
        shift(&mut path.tip);
        path.segments.iter_mut().flatten().for_each(shift);
        path.bounds =
            (path.bounds.0 + dx, path.bounds.1 + dy, path.bounds.2 + dx, path.bounds.3 + dy);
    }
    CanvasLayout { nodes, edges, width: bounds.2 - bounds.0, height: bounds.3 - bounds.1 }
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
        let against = layout.edges.iter().filter(|edge| !edge.path.rightwards).count();
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

    fn column_count(layout: &CanvasLayout) -> usize {
        layout.nodes.iter().map(|node| node.x as i32).collect::<BTreeSet<_>>().len()
    }

    #[test]
    fn a_hub_with_many_sources_folds_them_into_columns_on_a_bus() {
        let names: Vec<String> = (0..80).map(|n| format!("source{n:02}")).collect();
        let links: Vec<(&str, &str, &str)> =
            names.iter().map(|name| (name.as_str(), "userId", "users")).collect();
        let layout = layout(&graph(&links), "shop");
        let users = &layout.nodes[layout.index_of("users").unwrap()];

        assert_eq!(layout.nodes.len(), 81);
        assert_eq!(users.incoming, 80);
        assert!(column_count(&layout) > 2, "eighty sources in one column is unreadable");
        let one_column = 80.0 * (HEADER_HEIGHT + FIELD_HEIGHT + CARD_GAP as f32);
        assert!(layout.height < one_column / 2.0, "{} is still one tall strip", layout.height);
        // Folding moves a card outward; it never turns its edge around or sends it elsewhere.
        for edge in &layout.edges {
            assert!(edge.path.rightwards);
            assert_eq!(edge.path.tip, (users.x, users.y + HEADER_HEIGHT / 2.0));
        }
    }

    #[test]
    fn a_collection_with_many_lookups_folds_them_too() {
        let fields: Vec<String> = (0..70).map(|n| format!("lookup{n:02}Id")).collect();
        let names: Vec<String> = (0..70).map(|n| format!("lookup{n:02}")).collect();
        let links: Vec<(&str, &str, &str)> = fields
            .iter()
            .zip(&names)
            .map(|(field, name)| ("orders", field.as_str(), name.as_str()))
            .collect();
        let layout = layout(&graph(&links), "shop");
        let orders = &layout.nodes[layout.index_of("orders").unwrap()];

        assert!(column_count(&layout) > 2);
        assert!(layout.nodes.iter().all(|node| node.collection == "orders" || node.x > orders.x));
        assert!(layout.edges.iter().all(|edge| edge.path.rightwards));
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
        let users = &layout.nodes[layout.index_of("users").unwrap()];
        let edge = layout.edges.iter().find(|edge| edge.from.path == "userId").unwrap();

        // `userId` sorts after `items[].productId`, so it is the second row.
        assert_eq!(
            edge.path.start,
            (orders.x + CARD_WIDTH, orders.y + HEADER_HEIGHT + 1.5 * FIELD_HEIGHT)
        );
        assert_eq!(edge.path.tip, (users.x, users.y + HEADER_HEIGHT / 2.0));
        // The path itself stops short, leaving room for the arrowhead.
        assert_eq!(edge.path.segments.last().unwrap()[2], (users.x - ARROW, edge.path.tip.1));
        assert!(edge.path.rightwards);
    }

    /// Points along a path, close enough together that none can step over a card.
    fn sampled(path: &EdgePath) -> Vec<P> {
        let mut from = path.start;
        let mut points = Vec::new();
        for [a, b, to] in &path.segments {
            for step in 0..=24 {
                let t = step as f32 / 24.0;
                let u = 1.0 - t;
                let blend = |p: fn(&P) -> f32| {
                    u * u * u * p(&from)
                        + 3.0 * u * u * t * p(a)
                        + 3.0 * u * t * t * p(b)
                        + t * t * t * p(to)
                };
                points.push((blend(|p| p.0), blend(|p| p.1)));
            }
            from = *to;
        }
        points
    }

    #[test]
    fn an_edge_that_skips_a_rank_goes_around_the_card_in_it() {
        // `logs` reaches past `users` to `companies`. Drawn straight, it would cross `users`.
        let layout = layout(
            &graph(&[
                ("logs", "userId", "users"),
                ("users", "companyId", "companies"),
                ("logs", "companyId", "companies"),
            ]),
            "shop",
        );
        let users = &layout.nodes[layout.index_of("users").unwrap()];
        let long = layout
            .edges
            .iter()
            .find(|edge| edge.from.collection == "logs" && edge.to.collection == "companies")
            .unwrap();

        for (x, y) in sampled(&long.path) {
            let inside = x > users.x
                && x < users.x + CARD_WIDTH
                && y > users.y
                && y < users.y + users.height;
            assert!(!inside, "the path crosses `users` at ({x}, {y})");
        }
    }

    #[test]
    fn fields_joining_the_same_two_collections_share_one_trunk() {
        let layout = layout(
            &graph(&[
                ("orders", "buyerId", "users"),
                ("orders", "sellerId", "users"),
                ("orders", "shipping.courierId", "users"),
            ]),
            "shop",
        );

        assert_eq!(layout.edges.len(), 3);
        let starts: BTreeSet<i32> =
            layout.edges.iter().map(|edge| edge.path.start.1 as i32).collect();
        let tips: BTreeSet<(i32, i32)> = layout
            .edges
            .iter()
            .map(|edge| (edge.path.tip.0 as i32, edge.path.tip.1 as i32))
            .collect();
        assert_eq!(starts.len(), 3, "each leaves its own row");
        assert_eq!(tips.len(), 1, "and all arrive at the one header");
    }

    #[test]
    fn a_spline_rests_on_its_first_and_last_points() {
        let points = [(0.0, 0.0), (30.0, 0.0), (60.0, 80.0), (120.0, 80.0), (150.0, 40.0)];
        let segments = basis_spline(&points);

        assert_eq!(segments.last().unwrap()[2], (150.0, 40.0));
        // Each segment picks up where the one before left off, so the line has no gaps.
        assert!(segments.len() > points.len());
        assert_eq!(basis_spline(&points[..2]), vec![[(10.0, 0.0), (20.0, 0.0), (30.0, 0.0)]]);
        assert!(basis_spline(&[]).is_empty());
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
