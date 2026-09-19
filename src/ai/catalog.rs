//! The model catalogue: what each model costs, how much it can hold, what it can do.
//!
//! The data comes from models.dev (MIT). A pruned snapshot ships in the binary so the picker
//! is correct offline; the picker can also refresh from the live endpoint, which returns the
//! same shape, so one parser serves both.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::ai::errors::AiError;
use crate::ai::settings::AiProvider;

const BUNDLED: &str = include_str!("../../assets/ai-models.json");

pub const CATALOG_URL: &str = "https://models.dev/api.json";

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Limit {
    pub context: Option<u64>,
    pub output: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Modalities {
    #[serde(default)]
    pub input: Vec<String>,
    #[serde(default)]
    pub output: Vec<String>,
}

/// US dollars per million tokens.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Cost {
    pub input: Option<f64>,
    pub output: Option<f64>,
}

impl Cost {
    /// What a run of this model cost, in US dollars. `None` when the model lists no price,
    /// which is the honest answer for Ollama: it runs on your own machine.
    pub fn of(&self, input_tokens: u64, output_tokens: u64) -> Option<f64> {
        let (input, output) = (self.input?, self.output?);
        Some((input_tokens as f64 * input + output_tokens as f64 * output) / 1_000_000.0)
    }
}

/// Money spent, which is small enough that two decimals would read as free.
pub fn format_usd(dollars: f64) -> String {
    match dollars {
        _ if dollars >= 1.0 => format!("${dollars:.2}"),
        _ if dollars >= 0.01 => format!("${dollars:.3}"),
        _ if dollars >= 0.000_05 => format!("${dollars:.4}"),
        _ if dollars > 0.0 => "<$0.0001".to_string(),
        _ => "$0".to_string(),
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelInfo {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub tool_call: bool,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub structured_output: Option<bool>,
    #[serde(default)]
    pub release_date: Option<String>,
    /// `deprecated`, `beta`, or absent for generally available models.
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub limit: Limit,
    #[serde(default)]
    pub cost: Cost,
    #[serde(default)]
    pub modalities: Modalities,
    #[serde(default)]
    pub open_weights: bool,
}

impl ModelInfo {
    pub fn is_deprecated(&self) -> bool {
        self.status.as_deref() == Some("deprecated")
    }

    /// A text model this app can hold a tool-calling conversation with. Image and audio
    /// generators and live/realtime variants would only pad the picker.
    ///
    /// `serves_open_weights` is true for an aggregator like OpenRouter, which really does run
    /// Llama and Qwen for you; a first-party API listing them only means the lab published the
    /// weights, not that the endpoint serves them.
    pub fn is_chat_model(&self, serves_open_weights: bool) -> bool {
        self.tool_call
            && !self.is_deprecated()
            && (serves_open_weights || !self.open_weights)
            && self.modalities.output == ["text"]
            && self.modalities.input.iter().any(|input| input == "text")
    }

    /// "1M context · $2/$10 per Mtok", the line under a model in the picker.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(context) = self.limit.context {
            parts.push(format!("{} context", format_tokens(context)));
        }
        if let (Some(input), Some(output)) = (self.cost.input, self.cost.output) {
            parts.push(format!("{}/{} per Mtok", format_price(input), format_price(output)));
        }
        if !self.tool_call {
            parts.push("no tools".to_string());
        }
        if self.is_deprecated() {
            parts.push("deprecated".to_string());
        }
        parts.join(" · ")
    }
}

fn format_tokens(tokens: u64) -> String {
    match tokens {
        0..=999 => tokens.to_string(),
        1_000..=999_999 => format!("{}K", tokens / 1_000),
        _ => {
            let millions = tokens as f64 / 1_000_000.0;
            if (millions - millions.round()).abs() < 0.05 {
                format!("{}M", millions.round())
            } else {
                format!("{millions:.1}M")
            }
        }
    }
}

fn format_price(dollars: f64) -> String {
    if (dollars - dollars.round()).abs() < 0.005 {
        format!("${}", dollars.round())
    } else {
        format!("${dollars:.2}")
    }
}

#[derive(Debug, Deserialize)]
struct RawProvider {
    #[serde(default)]
    models: HashMap<String, ModelInfo>,
}

/// Models per provider, newest first. Unknown providers and fields are ignored, so the live
/// models.dev response parses the same way as the bundled snapshot.
#[derive(Debug, Default)]
pub struct ModelCatalog {
    providers: HashMap<String, Vec<ModelInfo>>,
}

impl ModelCatalog {
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        let raw: HashMap<String, RawProvider> = serde_json::from_str(json)?;
        let providers = raw
            .into_iter()
            .filter(|(key, _)| {
                AiProvider::ALL.iter().any(|provider| provider.catalog_key() == Some(key.as_str()))
            })
            .map(|(key, provider)| {
                let mut models: Vec<ModelInfo> = provider.models.into_values().collect();
                // Newest first, and a stable order for models sharing a release date.
                models.sort_by(|a, b| {
                    b.release_date.cmp(&a.release_date).then_with(|| a.id.cmp(&b.id))
                });
                (key, models)
            })
            .collect();
        Ok(Self { providers })
    }

    /// The snapshot compiled into the binary. Parsed once; a broken snapshot leaves the picker
    /// listing only the presets rather than taking the app down.
    pub fn bundled() -> Arc<Self> {
        static BUNDLED_CATALOG: OnceLock<Arc<ModelCatalog>> = OnceLock::new();
        BUNDLED_CATALOG
            .get_or_init(|| {
                Arc::new(Self::parse(BUNDLED).unwrap_or_else(|error| {
                    log::error!("Bundled model catalogue is unreadable: {error}");
                    Self::default()
                }))
            })
            .clone()
    }

    pub fn models(&self, provider: AiProvider) -> &[ModelInfo] {
        provider
            .catalog_key()
            .and_then(|key| self.providers.get(key))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn model(&self, provider: AiProvider, id: &str) -> Option<&ModelInfo> {
        self.models(provider).iter().find(|model| model.id == id)
    }

    pub fn is_empty(&self) -> bool {
        self.providers.values().all(Vec::is_empty)
    }

    /// The models this app can actually use: tool callers that are still supported.
    pub fn usable_model_ids(&self, provider: AiProvider) -> Vec<String> {
        self.models(provider)
            .iter()
            .filter(|model| model.is_chat_model(provider.is_aggregator()))
            .map(|model| model.id.clone())
            .collect()
    }

    /// models.dev's own shape, so a cached copy parses like the bundled snapshot.
    fn to_api_shape(&self) -> serde_json::Value {
        let providers: serde_json::Map<String, serde_json::Value> = self
            .providers
            .iter()
            .map(|(key, models)| {
                let models: serde_json::Map<String, serde_json::Value> = models
                    .iter()
                    .filter_map(|model| Some((model.id.clone(), serde_json::to_value(model).ok()?)))
                    .collect();
                (key.clone(), serde_json::json!({ "models": models }))
            })
            .collect();
        serde_json::Value::Object(providers)
    }
}

/// A catalogue cached on disk, with the ETag it came with.
#[derive(Debug, Deserialize, Serialize)]
struct CachedCatalog {
    #[serde(default)]
    etag: Option<String>,
    catalog: serde_json::Value,
}

fn cache_path() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("openmango").join("ai-models.json"))
}

/// The catalogue to start from: the last refresh when it is readable, else the snapshot.
pub fn load_cached_catalog() -> (Arc<ModelCatalog>, Option<String>) {
    let cached = cache_path()
        .filter(|path| path.exists())
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|json| serde_json::from_str::<CachedCatalog>(&json).ok())
        .and_then(|cached| {
            let catalog = ModelCatalog::parse(&cached.catalog.to_string()).ok()?;
            (!catalog.is_empty()).then_some((Arc::new(catalog), cached.etag))
        });
    match cached {
        Some((catalog, etag)) => (catalog, etag),
        None => (ModelCatalog::bundled(), None),
    }
}

fn store_cached_catalog(catalog: &ModelCatalog, etag: Option<&str>) {
    let Some(path) = cache_path() else { return };
    let cached = CachedCatalog { etag: etag.map(str::to_string), catalog: catalog.to_api_shape() };
    let write = std::fs::create_dir_all(path.parent().unwrap_or(&path))
        .and_then(|_| serde_json::to_string(&cached).map_err(std::io::Error::other))
        .and_then(|json| std::fs::write(&path, json));
    if let Err(error) = write {
        log::warn!("Could not cache the model catalogue: {error}");
    }
}

/// Fetch models.dev. `Ok(None)` means the catalogue we already have is current (HTTP 304).
pub async fn fetch_catalog(
    etag: Option<String>,
) -> Result<Option<(Arc<ModelCatalog>, Option<String>)>, AiError> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| AiError::Runtime(format!("failed to build HTTP client: {error}")))?;
    let mut request = http.get(CATALOG_URL);
    if let Some(etag) = etag.filter(|etag| !etag.trim().is_empty()) {
        request = request.header(reqwest::header::IF_NONE_MATCH, etag);
    }

    let response = request.send().await.map_err(|error| {
        if error.is_timeout() {
            AiError::Timeout(format!("Unable to reach {CATALOG_URL}"))
        } else {
            AiError::Network(format!("Unable to reach {CATALOG_URL}: {error}"))
        }
    })?;
    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(AiError::Network(format!("{CATALOG_URL} returned {}", response.status())));
    }

    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = response
        .text()
        .await
        .map_err(|error| AiError::Network(format!("Could not read {CATALOG_URL}: {error}")))?;
    let catalog = ModelCatalog::parse(&body)
        .map_err(|error| AiError::Runtime(format!("{CATALOG_URL} is not readable: {error}")))?;
    if catalog.is_empty() {
        return Err(AiError::Runtime(format!("{CATALOG_URL} listed no usable models")));
    }
    store_cached_catalog(&catalog, etag.as_deref());
    Ok(Some((Arc::new(catalog), etag)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::settings::ModelPreset;

    #[test]
    fn a_turn_is_priced_from_what_the_model_charges() {
        // Claude Sonnet money: $3 in, $15 out per million tokens.
        let cost = Cost { input: Some(3.0), output: Some(15.0) };
        let spent = cost.of(12_000, 800).expect("a priced model");
        assert!((spent - 0.048).abs() < 1e-9, "12k in and 800 out is 3.6c plus 1.2c");
        assert_eq!(format_usd(spent), "$0.048");

        // Ollama runs on your own machine, so there is no number to show.
        assert_eq!(Cost::default().of(12_000, 800), None);
    }

    #[test]
    fn small_change_never_rounds_away_to_nothing() {
        assert_eq!(format_usd(0.0), "$0");
        assert_eq!(format_usd(0.000_001), "<$0.0001");
        assert_eq!(format_usd(0.004_2), "$0.0042");
        assert_eq!(format_usd(0.42), "$0.420");
        assert_eq!(format_usd(12.5), "$12.50");
    }

    #[test]
    fn bundled_catalogue_covers_every_cloud_provider() {
        let catalog = ModelCatalog::bundled();
        for provider in AiProvider::ALL {
            let models = catalog.models(provider);
            if provider.catalog_key().is_some() {
                assert!(!models.is_empty(), "{} has no models", provider.label());
            } else {
                assert!(models.is_empty(), "{} should list models locally", provider.label());
            }
        }
    }

    /// The presets are curated ids; this fails when a snapshot refresh drops or deprecates one.
    #[test]
    fn every_preset_resolves_to_a_tool_calling_model() {
        let catalog = ModelCatalog::bundled();
        for provider in AiProvider::ALL {
            for preset in ModelPreset::ALL {
                let Some(id) = provider.preset_model(preset) else {
                    continue;
                };
                let model = catalog.model(provider, id).unwrap_or_else(|| {
                    panic!("{} {} missing: {id}", provider.label(), preset.label())
                });
                assert!(
                    model.is_chat_model(provider.is_aggregator()),
                    "{id} is not a usable chat model"
                );
            }
        }
    }

    #[test]
    fn models_are_listed_newest_first() {
        let catalog = ModelCatalog::bundled();
        let dates: Vec<_> = catalog
            .models(AiProvider::Anthropic)
            .iter()
            .filter_map(|model| model.release_date.clone())
            .collect();
        let mut sorted = dates.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(dates, sorted);
    }

    #[test]
    fn unknown_providers_and_fields_are_ignored() {
        let catalog = ModelCatalog::parse(
            r#"{"anthropic":{"id":"anthropic","name":"Anthropic","models":{
                 "m":{"id":"m","name":"M","tool_call":true,"npm":"x","limit":{"context":1000},
                      "modalities":{"input":["text"],"output":["text"]}}}},
                "some-other-provider":{"models":{}}}"#,
        )
        .expect("parse");
        let model = catalog.model(AiProvider::Anthropic, "m").expect("model");
        assert_eq!(model.summary(), "1K context");
    }

    #[test]
    fn summary_reads_like_a_price_tag() {
        let model = ModelInfo {
            id: "m".into(),
            name: "M".into(),
            tool_call: true,
            reasoning: true,
            structured_output: Some(true),
            release_date: None,
            status: None,
            limit: Limit { context: Some(1_000_000), output: Some(64_000) },
            cost: Cost { input: Some(0.3), output: Some(2.5) },
            modalities: Modalities { input: vec!["text".into()], output: vec!["text".into()] },
            open_weights: false,
        };
        assert_eq!(model.summary(), "1M context · $0.30/$2.50 per Mtok");
    }

    #[test]
    fn the_cache_shape_parses_back_into_the_same_catalogue() {
        let catalog = ModelCatalog::bundled();
        let round_tripped =
            ModelCatalog::parse(&catalog.to_api_shape().to_string()).expect("reparse");
        for provider in AiProvider::ALL {
            assert_eq!(
                round_tripped.usable_model_ids(provider),
                catalog.usable_model_ids(provider),
                "{}",
                provider.label()
            );
        }
    }
}
