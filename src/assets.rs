use std::borrow::Cow;

use gpui_kit::assets::Assets as ComponentAssets;
use gpui_kit::component::{Icon, IconNamed, icon_named};
use gpui_kit::{App, AssetSource, IntoElement, RenderOnce, Result, SharedString, Window};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
#[include = "logo/**/*.svg"]
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
