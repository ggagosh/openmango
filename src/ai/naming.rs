//! Naming a conversation.
//!
//! "how many audit logs are there per model, and should I index" is a fine thing to ask and a
//! poor thing to find later. The model that just answered it can say what it was about in four
//! words, so it does — once, on the cheapest model the provider offers, with the first exchange
//! as the whole input.

use crate::ai::provider::{AiGenerationRequest, generate_text};
use crate::ai::settings::{AiSettings, ModelPreset};

/// The most of the exchange the namer is shown. It only has to recognise the subject.
const MAX_INPUT_CHARS: usize = 1_200;
/// A title has to fit on one line beside a timestamp.
pub const MAX_TITLE_CHARS: usize = 48;

const PROMPT: &str = "You name conversations about MongoDB databases. Reply with a title of at \
                      most five words for the exchange below, in the user's own words where you \
                      can. No quotes, no punctuation at the end, no preamble — the title only.";

/// The settings to name with: the same provider, on its cheapest model.
///
/// A title is not worth the powerful model's price, and no provider is slower at four words than
/// it is at an answer. Where there are no presets — OpenRouter's catalogue, whatever Ollama is
/// serving — the model already in use does it.
fn namer_settings(settings: &AiSettings) -> AiSettings {
    let mut namer = settings.clone();
    if let Some(fast) = settings.provider.preset_model(ModelPreset::Fast) {
        namer.model = fast.to_string();
    }
    namer
}

/// Ask the model what a conversation was about. `None` when it will not say, which leaves the
/// first question as the name — never an error the user has to see.
pub async fn name_conversation(
    settings: &AiSettings,
    question: &str,
    answer: &str,
) -> Option<String> {
    let namer = namer_settings(settings);
    namer.validate_for_request().ok()?;

    let exchange = crate::helpers::truncate_chars(
        &format!("User: {question}\n\nAssistant: {answer}"),
        MAX_INPUT_CHARS,
    );
    let request = AiGenerationRequest {
        system_prompt: PROMPT.to_string(),
        history: Vec::new(),
        user_prompt: exchange,
        conversation_id: String::new(),
        // A title is not part of the conversation, and must not be stored as if it were.
        memory: None,
        context_tokens: None,
        price: None,
    };

    match generate_text(&namer, request).await {
        Ok(title) => clean_title(&title),
        Err(error) => {
            log::debug!("[ai-naming] could not name the conversation: {error}");
            None
        }
    }
}

/// Models answer a request for a title with a title, a quoted title, or a sentence about the
/// title. Take the first line, drop the decoration, and refuse anything that is clearly prose.
fn clean_title(raw: &str) -> Option<String> {
    let line = raw.lines().find(|line| !line.trim().is_empty())?;
    let title = line
        .trim()
        .trim_start_matches(['#', '-', '*', ' '])
        .trim_matches(['"', '\'', '`', ' '])
        .trim_end_matches(['.', ':'])
        .trim();
    if title.is_empty() || title.split_whitespace().count() > 10 {
        return None;
    }
    Some(crate::helpers::truncate_chars(title, MAX_TITLE_CHARS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::settings::AiProvider;

    #[test]
    fn a_title_survives_however_the_model_dressed_it_up() {
        assert_eq!(clean_title("Audit log indexing").as_deref(), Some("Audit log indexing"));
        assert_eq!(clean_title("\"Audit log indexing\"").as_deref(), Some("Audit log indexing"));
        assert_eq!(clean_title("## Audit log indexing.").as_deref(), Some("Audit log indexing"));
        assert_eq!(
            clean_title("Audit log indexing\n\nLet me know if you want another.").as_deref(),
            Some("Audit log indexing"),
            "only the first line is the title"
        );
    }

    #[test]
    fn prose_is_not_a_title() {
        assert_eq!(clean_title(""), None);
        assert_eq!(clean_title("   \n  "), None);
        assert_eq!(
            clean_title("Sure! Here is a short title that describes what you two talked about"),
            None,
            "a model that explains itself has not given a title"
        );
    }

    #[test]
    fn the_cheapest_model_does_the_naming() {
        let mut settings = AiSettings { provider: AiProvider::Anthropic, ..AiSettings::default() };
        settings.model = "claude-opus-5".to_string();
        assert_eq!(namer_settings(&settings).model, "claude-haiku-4-5");

        // A provider without presets names with whatever is already in use.
        let mut router = AiSettings { provider: AiProvider::OpenRouter, ..AiSettings::default() };
        router.model = "qwen/qwen3-max".to_string();
        assert_eq!(namer_settings(&router).model, "qwen/qwen3-max");
    }
}
