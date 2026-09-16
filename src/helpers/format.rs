pub fn format_number(value: u64) -> String {
    let raw = value.to_string();
    let mut out = String::with_capacity(raw.len() + raw.len() / 3);
    for (index, ch) in raw.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

pub fn format_bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if value == 0 {
        return "0 B".to_string();
    }

    let mut size = value as f64;
    let mut unit = 0usize;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }

    let formatted = if unit == 0 {
        format_number(value)
    } else if size < 10.0 {
        format!("{size:.1}")
    } else {
        format!("{size:.0}")
    };

    format!("{formatted} {}", UNITS[unit])
}

/// Shorten `text` to at most `max` characters, ending with "…" when cut. Safe for any UTF-8.
pub fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod truncate_tests {
    #[test]
    fn truncate_chars_never_splits_a_character() {
        assert_eq!(super::truncate_chars("héllo wörld", 6), "héllo…");
        assert_eq!(super::truncate_chars("short", 60), "short");
    }
}
