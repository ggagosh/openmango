use rig::completion::Message as RigMessage;

use crate::ai::blocks::ChatMessage;

const TOKENS_PER_CHAR_ESTIMATE: f64 = 0.30;
const RESERVED_OUTPUT_TOKENS: usize = 4_096;
const DEFAULT_MAX_CONTEXT_TOKENS: usize = 100_000;
const MIN_TAIL_MESSAGES: usize = 4;

fn estimate_tokens(chars: usize) -> usize {
    (chars as f64 * TOKENS_PER_CHAR_ESTIMATE) as usize
}

fn message_char_len(message: &ChatMessage) -> usize {
    message.content.chars().count()
}

fn transcript_char_len(message: &RigMessage) -> usize {
    serde_json::to_string(message).map(|json| json.chars().count()).unwrap_or(0)
}

/// Keep the model's own transcript inside its context window.
///
/// Whole turns are dropped from the front, never half of one: a tool result whose call was
/// dropped is a message most providers reject. A user message starts a turn.
pub fn trim_transcript(
    transcript: &mut Vec<RigMessage>,
    system_prompt_chars: usize,
    max_context_tokens: Option<usize>,
) {
    let context_limit = max_context_tokens.unwrap_or(DEFAULT_MAX_CONTEXT_TOKENS);
    let system_tokens = estimate_tokens(system_prompt_chars);
    if context_limit <= system_tokens + RESERVED_OUTPUT_TOKENS {
        return;
    }
    let available = context_limit - system_tokens - RESERVED_OUTPUT_TOKENS;

    loop {
        let chars: usize = transcript.iter().map(transcript_char_len).sum();
        if estimate_tokens(chars) <= available {
            return;
        }
        // The start of the next turn; without one there is a single turn left to keep.
        let Some(next_turn) = transcript
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, message)| matches!(message, RigMessage::User { .. }))
            .map(|(index, _)| index)
        else {
            return;
        };
        transcript.drain(..next_turn);
    }
}

pub fn trim_history_for_context(
    history: &mut Vec<ChatMessage>,
    system_prompt_chars: usize,
    max_context_tokens: Option<usize>,
) {
    if history.len() <= MIN_TAIL_MESSAGES + 1 {
        return;
    }

    let context_limit = max_context_tokens.unwrap_or(DEFAULT_MAX_CONTEXT_TOKENS);
    let system_tokens = estimate_tokens(system_prompt_chars);
    if context_limit <= system_tokens + RESERVED_OUTPUT_TOKENS {
        return;
    }
    let available = context_limit - system_tokens - RESERVED_OUTPUT_TOKENS;

    let total_chars: usize = history.iter().map(message_char_len).sum();
    if estimate_tokens(total_chars) <= available {
        return;
    }

    // Keep the newest turns, remove old middle history first.
    while history.len() > MIN_TAIL_MESSAGES + 1 {
        let chars: usize = history.iter().map(message_char_len).sum();
        if estimate_tokens(chars) <= available {
            break;
        }
        history.remove(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::blocks::ChatRole;

    #[test]
    fn a_long_transcript_drops_whole_turns_from_the_front() {
        let mut transcript = Vec::new();
        for index in 0..12 {
            transcript.push(RigMessage::user(format!("question {index} {}", "x".repeat(4_000))));
            transcript.push(RigMessage::assistant(format!("answer {index} {}", "y".repeat(4_000))));
        }

        trim_transcript(&mut transcript, 1_000, Some(20_000));

        assert!(transcript.len() < 24, "something was dropped");
        assert!(
            matches!(transcript.first(), Some(RigMessage::User { .. })),
            "a turn starts with what the user asked, never a dangling answer"
        );
    }

    #[test]
    fn a_transcript_that_fits_is_left_alone() {
        let mut transcript =
            vec![RigMessage::user("short question"), RigMessage::assistant("short answer")];
        let before = transcript.len();
        trim_transcript(&mut transcript, 100, Some(100_000));
        assert_eq!(transcript.len(), before);
    }

    #[test]
    fn trimming_removes_old_messages_when_over_budget() {
        let mut history = vec![];
        for idx in 0..20 {
            history.push(ChatMessage::new(
                ChatRole::User,
                format!("message-{idx}-{}", "x".repeat(80)),
            ));
        }

        trim_history_for_context(&mut history, 2_000, Some(5_000));
        assert!(history.len() < 20);
        assert!(!history.is_empty());
    }
}
