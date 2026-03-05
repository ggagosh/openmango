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
