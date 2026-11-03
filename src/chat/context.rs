//! User-selected context-window budgets, not vendor capability claims.
//!
//! These limits describe the whole model context, NOT `max_tokens` for one
//! response. `compaction` consumes this policy in the actual send pipeline.

pub const GPT_CONTEXT_LIMIT: u32 = 1_000_000;
pub const OTHER_CONTEXT_LIMIT: u32 = 256_000;

/// Match GPT model IDs, including namespaced router IDs, without mistaking a
/// provider name or an unrelated model containing "gpt" for a GPT model.
pub fn default_context_limit(model: &str) -> u32 {
    let model = model.trim().rsplit('/').next().unwrap_or_default();
    let model = model.to_ascii_lowercase();
    let is_gpt = ["gpt", "chatgpt"].iter().any(|prefix| {
        model.strip_prefix(*prefix).is_some_and(|tail| {
            tail.is_empty()
                || tail.starts_with('-')
                || tail.as_bytes().first().is_some_and(u8::is_ascii_digit)
        })
    });
    if is_gpt {
        GPT_CONTEXT_LIMIT
    } else {
        OTHER_CONTEXT_LIMIT
    }
}

/// Empty restores the family default. K/M use decimal token units; only whole,
/// positive token counts are accepted, with checked multiplication.
pub fn parse_context_limit(text: &str) -> Result<Option<u32>, &'static str> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let (number, multiplier) = match text.as_bytes().last() {
        Some(b'k' | b'K') => (&text[..text.len() - 1], 1_000u32),
        Some(b'm' | b'M') => (&text[..text.len() - 1], 1_000_000u32),
        _ => (text, 1u32),
    };
    let number = number.trim();
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("請輸入正整數 tokens，或使用 128K、1M；留空使用預設。");
    }
    number
        .parse::<u32>()
        .ok()
        .and_then(|value| value.checked_mul(multiplier))
        .filter(|value| *value > 0)
        .map(Some)
        .ok_or("上限必須介於 1 與 4,294,967,295 tokens。")
}

pub fn context_limit_label(tokens: u32) -> String {
    if tokens > 0 && tokens % 1_000_000 == 0 {
        format!("{}M", tokens / 1_000_000)
    } else if tokens > 0 && tokens % 1_000 == 0 {
        format!("{}K", tokens / 1_000)
    } else {
        tokens.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_defaults_follow_user_policy_including_router_ids() {
        for model in [
            "gpt-4o",
            "gpt-6.1-sol",
            "GPT-5.1",
            "gpt4",
            "chatgpt-4o-latest",
            "openai/gpt-4.1",
            " gpt-5 ",
        ] {
            assert_eq!(default_context_limit(model), 1_000_000, "{model}");
        }
        for model in [
            "",
            "claude-sonnet-4",
            "gemini-2.5-pro",
            "o3",
            "codex-mini-latest",
            "my-gpt-model",
            "gptoss",
            "gpt-provider/llama-3",
            "模型",
        ] {
            assert_eq!(default_context_limit(model), 256_000, "{model}");
        }
    }

    #[test]
    fn context_limit_parser_accepts_decimal_token_units_or_default() {
        for (text, expected) in [
            ("", None),
            ("  ", None),
            ("128K", Some(128_000)),
            ("256k", Some(256_000)),
            ("1M", Some(1_000_000)),
            ("2m", Some(2_000_000)),
            (" 1000 ", Some(1000)),
            ("4294967295", Some(u32::MAX)),
        ] {
            assert_eq!(parse_context_limit(text).unwrap(), expected, "{text}");
        }
        for text in [
            "0",
            "0K",
            "-1",
            "+1",
            "1.5M",
            "K",
            "1e6",
            "256KiB",
            "一萬",
            "128,000",
            "4294967296",
            "4294968K",
            "4295M",
        ] {
            assert!(parse_context_limit(text).is_err(), "{text}");
        }
    }

    #[test]
    fn labels_do_not_round_away_exact_limits() {
        for value in [1, 128_000, 256_000, 1_000_000, 1_048_576, u32::MAX] {
            assert_eq!(
                parse_context_limit(&context_limit_label(value)).unwrap(),
                Some(value)
            );
        }
    }
}
