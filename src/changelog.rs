//! "What's New" dialog shown after app updates.

use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::WindowExt as _;
use gpui_component::dialog::Dialog;
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::tag::Tag;
use gpui_component::{Icon, IconName, Sizable as _};

use crate::components::Button;
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
    let label: SharedString = entry.category.clone().into();

    match entry.category.as_str() {
        "Added" => Tag::success()
            .small()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(Icon::new(IconName::Plus).xsmall())
                    .child(label),
            )
            .into_any_element(),
        "Fixed" => Tag::info()
            .small()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(Icon::new(IconName::Check).xsmall())
                    .child(label),
            )
            .into_any_element(),
        "Changed" => Tag::secondary()
            .small()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(Icon::new(IconName::Redo).xsmall())
                    .child(label),
            )
            .into_any_element(),
        "Removed" => Tag::danger()
            .small()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(Icon::new(IconName::Delete).xsmall())
                    .child(label),
            )
            .into_any_element(),
        _ => Tag::secondary().small().child(label).into_any_element(),
    }
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
                    .child(
                        div().flex_shrink_0().mt(px(7.0)).size(px(5.0)).rounded(px(2.5)).bg(muted),
                    )
                    .child(div().text_sm().child(text))
            }),
        ))
        .into_any_element()
}

/// Show the "What's New" dialog for the current build.
///
/// Persists `OPENMANGO_GIT_SHA` as `last_seen_version` so the dialog only
/// appears once per unique build (works for both stable releases and nightlies).
pub fn show_whats_new_dialog(
    state: Entity<crate::state::AppState>,
    window: &mut Window,
    cx: &mut App,
) {
    let build_id = env!("OPENMANGO_GIT_SHA");
    let pkg_version = env!("CARGO_PKG_VERSION");

    // Try the exact version section first, fall back to [Unreleased]
    let (section_md, title) = if let Some(section) = extract_version_section(pkg_version) {
        let title: SharedString = format!("What's New in v{pkg_version}").into();
        (section, title)
    } else if let Some(section) = extract_unreleased_section() {
        let title: SharedString = "What's New".into();
        (section, title)
    } else {
        // No changelog content at all — silently update and skip
        state.update(cx, |state, _cx| {
            state.settings.last_seen_version = build_id.to_string();
            state.save_settings();
        });
        return;
    };

    let sections = parse_changelog_sections(&section_md);
    let build_id_owned = build_id.to_string();

    window.open_dialog(cx, move |dialog: Dialog, _window: &mut Window, cx: &mut App| {
        let state_clone = state.clone();
        let build_id = build_id_owned.clone();

        let section_elements: Vec<AnyElement> =
            sections.iter().map(|entry| render_section(entry, cx)).collect();

        dialog.title(title.clone()).min_w(px(600.0)).keyboard(false).child(
            div()
                .flex()
                .flex_col()
                .gap(spacing::lg())
                .p(spacing::lg())
                .on_key_down({
                    let state_clone = state_clone.clone();
                    let build_id = build_id.clone();
                    move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
                        let key = event.keystroke.key.to_ascii_lowercase();
                        if key == "escape" || key == "enter" || key == "return" {
                            cx.stop_propagation();
                            state_clone.update(cx, |state, _cx| {
                                state.settings.last_seen_version = build_id.clone();
                                state.save_settings();
                            });
                            window.close_dialog(cx);
                        }
                    }
                })
                .child(
                    div()
                        .max_h(px(400.0))
                        .overflow_y_scrollbar()
                        .flex()
                        .flex_col()
                        .text_color(cx.theme().secondary_foreground)
                        .children(section_elements),
                )
                .child(div().flex().justify_end().child(
                    Button::new("whats-new-ok").primary().label("OK").on_click({
                        let state_clone = state_clone.clone();
                        let build_id = build_id.clone();
                        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                            state_clone.update(cx, |state, _cx| {
                                state.settings.last_seen_version = build_id.clone();
                                state.save_settings();
                            });
                            window.close_dialog(cx);
                        }
                    }),
                )),
        )
    });
}
