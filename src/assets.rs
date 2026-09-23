use std::borrow::Cow;

use gpui_kit::assets::Assets as ComponentAssets;
use gpui_kit::component::{Icon, IconNamed, icon_named};
use gpui_kit::{App, AssetSource, IntoElement, RenderOnce, Result, SharedString, Window};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
#[include = "logo/**/*.png"]
#[include = "fonts/**/*.ttf"]
#[include = "fonts/**/*.otf"]
pub struct EmbeddedAssets;

pub struct Assets;

icon_named!(AppIcon, "assets/icons");

impl RenderOnce for AppIcon {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        Icon::new(self)
    }
}

pub fn embedded_fonts() -> Vec<Cow<'static, [u8]>> {
    EmbeddedAssets::iter()
        .filter(|path| path.starts_with("fonts/"))
        .filter(|path| path.ends_with(".ttf") || path.ends_with(".otf"))
        .filter_map(|path| EmbeddedAssets::get(path.as_ref()).map(|file| file.data))
        .collect()
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }

        if let Some(file) = EmbeddedAssets::get(path) {
            return Ok(Some(file.data));
        }

        let component_assets = ComponentAssets;
        component_assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let component_assets = ComponentAssets;
        let mut entries = component_assets.list(path)?;

        for entry in EmbeddedAssets::iter().filter(|p| p.starts_with(path)) {
            entries.push(entry.into());
        }

        entries.sort();
        entries.dedup();
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The kit loads only its default icons at runtime; any other Lucide icon must be copied
    /// into `assets/icons`. Every `app_icon("…")` and `icons/….svg` named in the source must load.
    #[test]
    fn every_icon_named_in_the_source_loads() {
        fn sources(dir: &std::path::Path, found: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    sources(&path, found);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    found.push(std::fs::read_to_string(path).unwrap());
                }
            }
        }
        let mut files = Vec::new();
        sources(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
        let mut names = std::collections::BTreeSet::new();
        for file in &files {
            for (prefix, suffix) in [("app_icon(\"", "\")"), ("\"icons/", ".svg\"")] {
                for part in file.split(prefix).skip(1) {
                    if let Some(name) = part.split(suffix).next()
                        && !name.is_empty()
                        && name
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                    {
                        names.insert(name.to_string());
                    }
                }
            }
        }
        assert!(names.contains("git-compare-arrows"), "the scan finds icon names");
        let missing: Vec<_> = names
            .iter()
            .filter(|name| !matches!(Assets.load(&format!("icons/{name}.svg")), Ok(Some(_))))
            .collect();
        assert!(missing.is_empty(), "icons that do not load: {missing:?}");
    }
}
