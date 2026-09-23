//! "What's New": a release's highlights up front, every other change one click away.

use std::collections::HashSet;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::text::{TextView, TextViewStyle};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use sha2::{Digest as _, Sha256};

use crate::state::AppState;
use crate::theme::spacing;

const CHANGELOG: &str = include_str!("../CHANGELOG.md");
const FULL_CHANGELOG_URL: &str = "https://github.com/ggagosh/openmango/blob/main/CHANGELOG.md";

/// `SHOW_NOTES=1 just dev` opens What's New with the notes about to ship (`[Unreleased]`);
/// `SHOW_NOTES=0.3.0` opens a released version's. `OPENMANGO_SHOW_CHANGELOG` only forces it open.
fn requested_section() -> Option<String> {
    let value = std::env::var("SHOW_NOTES").ok()?;
    let value = value.trim();
    Some(if value.is_empty() || value == "1" { "Unreleased".into() } else { value.into() })
}

pub fn forced() -> bool {
    requested_section().is_some() || std::env::var("OPENMANGO_SHOW_CHANGELOG").is_ok()
}

struct Highlight {
    title: Option<String>,
    text: String,
}

struct Group {
    title: String,
    items: Vec<String>,
}

struct Release {
    title: SharedString,
    subtitle: SharedString,
    highlights: Vec<Highlight>,
    groups: Vec<Group>,
    /// Changes when what the screen leads with changes; startup opens it only then.
    key: String,
}

/// The `## [name] - date` section: its date, if any, and its body.
fn section<'a>(changelog: &'a str, name: &str) -> Option<(Option<&'a str>, String)> {
    let target = format!("## [{name}]");
    let mut lines = changelog.lines();
    let header = lines.find(|line| line.trim().starts_with(&target))?;
    let date = header.trim()[target.len()..].trim().strip_prefix('-').map(str::trim);
    let body: Vec<_> = lines.take_while(|line| !line.trim().starts_with("## [")).collect();
    let body = body.join("\n").trim().to_string();
    (!body.is_empty()).then_some((date.filter(|date| !date.is_empty()), body))
}

/// `### Category` groups of `- item` lines.
fn groups(body: &str) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for line in body.lines().map(str::trim) {
        if let Some(title) = line.strip_prefix("### ") {
            groups.push(Group { title: title.trim().into(), items: Vec::new() });
        } else if let (Some(item), Some(group)) = (line.strip_prefix("- "), groups.last_mut()) {
            group.items.push(item.into());
        }
    }
    groups.retain(|group| !group.items.is_empty());
    groups
}

/// `**Title.** Sentence.` reads as a title over its sentence.
fn highlight(item: &str) -> Highlight {
    let split = item.strip_prefix("**").and_then(|rest| rest.split_once("**"));
    match split {
        Some((title, text)) => Highlight {
            title: Some(title.trim().trim_end_matches('.').into()),
            text: text.trim().into(),
        },
        None => Highlight { title: None, text: item.into() },
    }
}

fn notes_key(highlights: &[Group], body: &str) -> String {
    let source = match highlights.first() {
        Some(group) => group.items.join("\n"),
        None => body.to_string(),
    };
    let digest = Sha256::digest(source.as_bytes());
    let hex: String = digest.iter().take(8).map(|byte| format!("{byte:02x}")).collect();
    format!("notes-{hex}")
}

fn release(changelog: &str, name: &str, title: String, subtitle: String) -> Option<Release> {
    let (date, body) = section(changelog, name)?;
    let (highlights, groups): (Vec<_>, Vec<_>) =
        groups(&body).into_iter().partition(|group| group.title == "Highlights");
    let subtitle = match date {
        Some(date) if subtitle.is_empty() => format!("Released {date}"),
        _ => subtitle,
    };
    Some(Release {
        key: notes_key(&highlights, &body),
        title: title.into(),
        subtitle: subtitle.into(),
        highlights: highlights
            .into_iter()
            .flat_map(|group| group.items)
            .map(|item| highlight(&item))
            .collect(),
        groups,
    })
}

/// Nightly shows what is about to ship; stable shows its own version.
fn current() -> Option<Release> {
    let version = env!("CARGO_PKG_VERSION");
    let build = env!("OPENMANGO_GIT_SHA");
    let nightly = option_env!("OPENMANGO_RELEASE_CHANNEL")
        .unwrap_or("stable")
        .eq_ignore_ascii_case("nightly");
    let unreleased = || {
        let subtitle = if nightly && !build.is_empty() {
            format!("Nightly build {}", &build[..7.min(build.len())])
        } else {
            "Not released yet".to_string()
        };
        release(CHANGELOG, "Unreleased", "What's New".into(), subtitle)
    };
    let version_release =
        |name: &str| release(CHANGELOG, name, format!("What's New in {name}"), String::new());
    match requested_section() {
        Some(name) if name == "Unreleased" => unreleased(),
        Some(name) => version_release(&name),
        None if nightly => unreleased().or_else(|| version_release(version)),
        None => version_release(version).or_else(unreleased),
    }
}

/// What startup compares with the last notes the user saw.
pub fn current_key() -> String {
    current().map(|release| release.key).unwrap_or_default()
}

fn markdown_style(cx: &App) -> TextViewStyle {
    TextViewStyle {
        paragraph_gap: rems(0.5),
        heading_base_font_size: px(14.0),
        inline_code: HighlightStyle {
            color: Some(crate::theme::colors::syntax_key(cx)),
            background_color: Some(cx.theme().accent.opacity(0.55)),
            ..Default::default()
        },
        is_dark: cx.theme().mode.is_dark(),
        ..TextViewStyle::default()
    }
}

pub struct ChangelogView {
    state: Entity<AppState>,
    release: Option<Release>,
    /// Groups opened by the user; without highlights every group starts open.
    expanded: HashSet<usize>,
    _subscriptions: Vec<Subscription>,
}

impl ChangelogView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.observe(&state, |_, _, cx| cx.notify())];
        Self { state, release: current(), expanded: HashSet::new(), _subscriptions: subscriptions }
    }

    #[cfg(test)]
    fn from_notes(state: Entity<AppState>, notes: &str, cx: &mut Context<Self>) -> Self {
        let mut view = Self::new(state, cx);
        view.release = release(notes, "Unreleased", "What's New".into(), String::new());
        view
    }
}

impl Render for ChangelogView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = &self.state;
        let muted = cx.theme().muted_foreground;
        let style = markdown_style(cx);
        let (title, subtitle) = match &self.release {
            Some(release) => (release.title.clone(), release.subtitle.clone()),
            None => ("What's New".into(), SharedString::default()),
        };
        let mut page = div()
            .w_full()
            .max_w(px(640.0))
            .mx_auto()
            .px(px(32.0))
            .pt(px(32.0))
            .pb(px(80.0))
            .flex()
            .flex_col()
            .gap(spacing::lg())
            .text_sm()
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap(spacing::md())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(spacing::xs())
                            .child(div().text_xl().font_weight(FontWeight::BOLD).child(title))
                            .when(!subtitle.is_empty(), |header| {
                                header.child(div().text_color(muted).child(subtitle))
                            }),
                    )
                    .child(
                        Button::new("full-changelog")
                            .ghost()
                            .small()
                            .icon(crate::views::compare::app_icon("square-arrow-out-up-right"))
                            .label("Full changelog")
                            .on_click(|_, _, cx| cx.open_url(FULL_CHANGELOG_URL)),
                    ),
            )
            .child(div().h(px(1.0)).bg(cx.theme().border));
        let Some(release) = &self.release else {
            return div()
                .size_full()
                .overflow_y_scrollbar()
                .child(page.child(div().text_color(muted).child("No notes for this build.")));
        };

        for (index, highlight) in release.highlights.iter().enumerate() {
            page = page.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .when_some(highlight.title.clone(), |block, title| {
                        block
                            .child(div().text_base().font_weight(FontWeight::SEMIBOLD).child(title))
                    })
                    .child(
                        div().text_color(cx.theme().secondary_foreground).child(
                            TextView::markdown(("highlight", index), highlight.text.clone())
                                .style(style.clone()),
                        ),
                    ),
            );
        }

        let collapsible = !release.highlights.is_empty();
        let mut groups = div().flex().flex_col().gap(spacing::sm());
        for (index, group) in release.groups.iter().enumerate() {
            let expanded = !collapsible || self.expanded.contains(&index);
            let label = format!("{} ({})", group.title, group.items.len());
            let heading = if collapsible {
                Button::new(("notes-group", index))
                    .ghost()
                    .small()
                    .icon(if expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                    .label(label)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if !view.expanded.remove(&index) {
                            view.expanded.insert(index);
                        }
                        cx.notify();
                    }))
                    .into_any_element()
            } else {
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(muted)
                    .child(label)
                    .into_any_element()
            };
            let list = group.items.iter().map(|item| format!("- {item}\n")).collect::<String>();
            groups = groups.child(
                div().flex().flex_col().gap(spacing::xs()).child(div().flex().child(heading)).when(
                    expanded,
                    |block| {
                        block.child(div().text_color(cx.theme().secondary_foreground).child(
                            TextView::markdown(("notes-list", index), list).style(style.clone()),
                        ))
                    },
                ),
            );
        }
        if collapsible {
            page = page.child(div().h(px(1.0)).bg(cx.theme().border));
        }

        div().size_full().overflow_y_scrollbar().child(page.child(groups))
    }
}

/// Open the changelog tab and record these notes as seen.
///
/// Called from startup (when the notes changed) and from the action bar command.
/// If a workspace restore is pending the tab is deferred until after restore
/// finishes (otherwise `restore_tabs_from_workspace` would wipe it).
pub fn open_changelog_tab(state: Entity<AppState>, cx: &mut App) {
    state.update(cx, |state, cx| {
        state.settings.last_seen_version = current_key();
        state.save_settings();
        if state.workspace_restore_pending {
            state.changelog_pending = true;
        } else {
            state.open_changelog_tab(cx);
        }
    });
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings gpui_kit's own `test` macro over `#[test]`.
    use super::{ChangelogView, release};

    const NOTES: &str = "\
# Changelog

## [Unreleased]

### Highlights
- **Compare and sync.** Compare two collections, then sync what you choose.
- Plain highlight with `code`.

### Added
- One
- Two

### Fixed
- Three

## [0.3.0] - 2026-09-14

### Added
- Older
";

    #[test]
    fn highlights_lead_and_other_groups_follow() {
        let notes = release(NOTES, "Unreleased", "What's New".into(), "Nightly".into()).unwrap();
        assert_eq!(notes.highlights.len(), 2);
        assert_eq!(notes.highlights[0].title.as_deref(), Some("Compare and sync"));
        assert_eq!(notes.highlights[0].text, "Compare two collections, then sync what you choose.");
        assert_eq!(notes.highlights[1].title, None);
        let titles: Vec<_> =
            notes.groups.iter().map(|g| (g.title.as_str(), g.items.len())).collect();
        assert_eq!(titles, [("Added", 2), ("Fixed", 1)]);
        let older = release(NOTES, "0.3.0", "What's New in 0.3.0".into(), String::new()).unwrap();
        assert_eq!(older.subtitle.as_ref(), "Released 2026-09-14");
        assert!(older.highlights.is_empty());
        assert!(release(NOTES, "9.9.9", String::new(), String::new()).is_none());
    }

    #[test]
    fn the_key_follows_highlights_not_every_line() {
        let key =
            |notes: &str| release(notes, "Unreleased", String::new(), String::new()).unwrap().key;
        let base = key(NOTES);
        assert_eq!(key(&NOTES.replace("- Three", "- Three and a half")), base, "a new fix alone");
        assert_ne!(key(&NOTES.replace("then sync", "and sync")), base, "a changed highlight");
        assert!(!base.contains('.'), "never mistaken for a legacy semver value");
    }

    #[gpui_kit::test]
    fn highlights_show_and_groups_open_on_click(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{AppContext as _, px, size};
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::apply_design_tokens(cx);
        });
        let directory = tempfile::tempdir().unwrap();
        let state = cx.new(|_| {
            crate::state::AppState::with_config(
                std::sync::Arc::new(crate::connection::ConnectionManager::new()),
                crate::state::ConfigManager::with_config_dir(directory.path().into()),
            )
        });
        let mut view = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let changelog = cx.new(|cx| ChangelogView::from_notes(state.clone(), NOTES, cx));
            view = Some(changelog.clone());
            gpui_kit::component::Root::new(changelog, window, cx).bordered(false)
        });
        let view = view.unwrap();
        cx.simulate_resize(size(px(900.0), px(700.0)));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let group = cx
            .update(|window, _| gpui_kit::base::test_support::snapshots(window))
            .into_iter()
            .find(|node| node.path().last() == Some(&("notes-group", 0usize).into()))
            .expect("Added starts as a collapsed group");
        cx.simulate_click(group.bounds().center(), Default::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        view.read_with(cx, |view, _| assert!(view.expanded.contains(&0)));
    }
}
