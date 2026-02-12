//! "What's New" changelog view (shown as a tab).

use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::Sizable as _;
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::tag::Tag;

use crate::state::AppState;
use crate::theme::spacing;

const CHANGELOG: &str = include_str!("../CHANGELOG.md");

struct ChangelogEntry {
    category: String,
    items: Vec<String>,
}

/// Extract the changelog section for a specific version.
///
/// Finds the `## [version]` header and returns everything until the next
/// `## [` header or end of file. Returns `None` if the version isn't found.
fn extract_version_section(version: &str) -> Option<String> {
    let target = format!("## [{version}]");
    let mut lines = CHANGELOG.lines();

    // Find the target version header
    lines.find(|line| {
        let trimmed = line.trim();
        trimmed.starts_with(&target)
    })?;

    // Collect lines until the next ## [ header
    let mut section = String::new();
    for line in lines {
        if line.trim().starts_with("## [") {
            break;
        }
        section.push_str(line);
        section.push('\n');
    }

    let trimmed = section.trim();
    if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
}

/// Extract the `[Unreleased]` changelog section.
fn extract_unreleased_section() -> Option<String> {
    extract_version_section("Unreleased")
}

/// Parse a changelog section body into structured entries grouped by category.
fn parse_changelog_sections(md: &str) -> Vec<ChangelogEntry> {
    let mut entries: Vec<ChangelogEntry> = Vec::new();
    let mut current_category: Option<String> = None;
    let mut current_items: Vec<String> = Vec::new();

    for line in md.lines() {
        let trimmed = line.trim();
        if let Some(header) = trimmed.strip_prefix("### ") {
            // Flush previous category
            if let Some(cat) = current_category.take()
                && !current_items.is_empty()
            {
                entries.push(ChangelogEntry {
                    category: cat,
                    items: std::mem::take(&mut current_items),
                });
            }
            current_category = Some(header.trim().to_string());
        } else if let Some(item) = trimmed.strip_prefix("- ") {
            current_items.push(item.to_string());
        }
    }

    // Flush last category
    if let Some(cat) = current_category
        && !current_items.is_empty()
    {
        entries.push(ChangelogEntry { category: cat, items: current_items });
    }

    entries
}

/// Build a Tag element for a changelog category.
fn category_tag(entry: &ChangelogEntry, _cx: &App) -> AnyElement {
    let (emoji, label) = match entry.category.as_str() {
        "Added" => ("✨", "Added"),
        "Fixed" => ("🐛", "Fixed"),
        "Changed" => ("🔄", "Changed"),
        "Removed" => ("🗑️", "Removed"),
        "Improved" => ("⚡", "Improved"),
        "Security" => ("🔒", "Security"),
        _ => ("📋", entry.category.as_str()),
    };
    let text: SharedString = format!("{emoji}  {label}").into();

    let tag = match entry.category.as_str() {
        "Added" => Tag::success(),
        "Fixed" => Tag::info(),
        "Changed" | "Improved" => Tag::secondary(),
        "Removed" => Tag::danger(),
        "Security" => Tag::warning(),
        _ => Tag::secondary(),
    };

    tag.small().child(text).into_any_element()
}

/// Render a single changelog section (tag header + bullet items).
fn render_section(entry: &ChangelogEntry, cx: &App) -> AnyElement {
    let muted = cx.theme().muted_foreground;

    div()
        .flex()
        .flex_col()
        .gap(spacing::sm())
        .pb(spacing::lg())
        .child(div().flex().child(category_tag(entry, cx)))
        .child(div().flex().flex_col().gap(spacing::xs()).pl(spacing::sm()).children(
            entry.items.iter().map(|item| {
                let text: SharedString = item.clone().into();
                div()
                    .flex()
                    .items_start()
                    .gap(spacing::sm())
                    .min_w_0()
                    .child(
                        div().flex_shrink_0().mt(px(7.0)).size(px(5.0)).rounded(px(2.5)).bg(muted),
                    )
                    .child(div().flex_1().min_w_0().text_sm().whitespace_normal().child(text))
            }),
        ))
        .into_any_element()
}

// ============================================================================
// ChangelogView — GPUI entity rendered as a tab
// ============================================================================

pub struct ChangelogView {
    state: Entity<AppState>,
    title: SharedString,
    sections: Vec<ChangelogEntry>,
    _subscriptions: Vec<Subscription>,
}

impl ChangelogView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.observe(&state, |_, _, cx| cx.notify())];

        let build_id = env!("OPENMANGO_GIT_SHA");
        let pkg_version = env!("CARGO_PKG_VERSION");
        let release_channel = option_env!("OPENMANGO_RELEASE_CHANNEL").unwrap_or("stable");
        let is_nightly = release_channel.eq_ignore_ascii_case("nightly");

        let (section_md, title) = if is_nightly {
            if let Some(section) = extract_unreleased_section() {
                let title: SharedString = if build_id.is_empty() {
                    "What's New (Nightly)".into()
                } else {
                    let short_sha = &build_id[..7.min(build_id.len())];
                    format!("What's New (Nightly {short_sha})").into()
                };
                (Some(section), title)
            } else if let Some(section) = extract_version_section(pkg_version) {
                let title: SharedString = format!("What's New in v{pkg_version}").into();
                (Some(section), title)
            } else {
                (None, SharedString::from("What's New"))
            }
        } else if let Some(section) = extract_version_section(pkg_version) {
            let title: SharedString = format!("What's New in v{pkg_version}").into();
            (Some(section), title)
        } else if let Some(section) = extract_unreleased_section() {
            let title: SharedString = "What's New".into();
            (Some(section), title)
        } else {
            (None, SharedString::from("What's New"))
        };

        let sections = section_md.map(|md| parse_changelog_sections(&md)).unwrap_or_default();

        Self { state, title, sections, _subscriptions: subscriptions }
    }
}

impl Render for ChangelogView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = &self.state;
        let section_elements: Vec<AnyElement> =
            self.sections.iter().map(|entry| render_section(entry, cx)).collect();

        let pkg_version = env!("CARGO_PKG_VERSION");
        let subtitle: SharedString = format!("v{pkg_version}").into();

        div().flex().flex_col().flex_1().size_full().overflow_y_scrollbar().child(
            div()
                .max_w(px(860.0))
                .mx_auto()
                .px(px(32.0))
                .pt(px(32.0))
                .pb(px(80.0))
                .flex()
                .flex_col()
                .gap(px(24.0))
                // Header
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(spacing::xs())
                        .child(
                            div().text_xl().font_weight(FontWeight::BOLD).child(self.title.clone()),
                        )
                        .child(
                            div().text_sm().text_color(cx.theme().muted_foreground).child(subtitle),
                        ),
                )
                // Divider
                .child(div().h(px(1.0)).bg(cx.theme().border))
                // Sections
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(spacing::sm())
                        .text_color(cx.theme().secondary_foreground)
                        .children(section_elements),
                ),
        )
    }
}

// ============================================================================
// Public API — open changelog tab + persist last_seen_version
// ============================================================================

/// Open the changelog tab and persist `last_seen_version`.
///
/// Called from startup (when build SHA changes) and from the action bar command.
/// If a workspace restore is pending the tab is deferred until after restore
/// finishes (otherwise `restore_tabs_from_workspace` would wipe it).
pub fn open_changelog_tab(state: Entity<AppState>, cx: &mut App) {
    let build_id = env!("OPENMANGO_GIT_SHA");
    state.update(cx, |state, cx| {
        state.settings.last_seen_version = build_id.to_string();
        state.save_settings();
        if state.workspace_restore_pending {
            state.changelog_pending = true;
        } else {
            state.open_changelog_tab(cx);
        }
    });
}
