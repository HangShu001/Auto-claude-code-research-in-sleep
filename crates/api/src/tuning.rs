//! v0.4.28: request tuning shared by every Anthropic-channel caller (the main
//! session and subagents) — reasoning effort, the output-token override, and
//! the accumulation of streamed tool input.
//!
//! All of it is opt-in: with `ARIS_REASONING_EFFORT` and `ARIS_MAX_TOKENS`
//! unset, requests are byte-identical to v0.4.27.

use serde_json::{json, Value};

/// The env var the `OpenAI` channel has always honoured; v0.4.28 makes the
/// Anthropic channel honour it too (#446).
pub const REASONING_EFFORT_ENV: &str = "ARIS_REASONING_EFFORT";
/// Optional override of the per-model output cap on the Anthropic channel.
pub const MAX_TOKENS_ENV: &str = "ARIS_MAX_TOKENS";

/// Which effort levels a model accepts through `output_config.effort`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffortSupport {
    /// `low` / `medium` / `high` / `xhigh` / `max`.
    Full,
    /// `low` / `medium` / `high` / `max` — the 4.6 family predates `xhigh`.
    WithoutXhigh,
    /// `low` / `medium` / `high`, and no adaptive thinking (Opus 4.5).
    EffortOnly,
    /// The model errors on `effort` (Haiku 4.5, older Claude models) or is
    /// not a Claude model at all (an Anthropic-compatible endpoint serving
    /// something else).
    Unsupported,
}

/// What to do with the reasoning-effort setting for one request.
#[derive(Debug, Clone, PartialEq)]
pub enum EffortDecision {
    /// Nothing configured: send no field (today's request).
    Unset,
    /// Add these fields to the request (`thinking` only where the model
    /// takes adaptive thinking).
    Send {
        thinking: Option<Value>,
        output_config: Value,
    },
    /// Configured, but this model cannot take it: send no field.
    UnsupportedModel,
    /// Configured with a value that is not an effort level: send no field.
    InvalidValue(String),
}

/// Model families that accept `output_config.effort`, matched on the model id
/// with any provider prefix (`anthropic/…`, `anthropic.…`) removed. A family
/// matches the bare id and its dated / suffixed snapshots
/// (`claude-opus-4-8-20260115`, `claude-opus-4-8@20260115`), never a longer
/// number (`claude-opus-4-80`).
const FULL_EFFORT_FAMILIES: &[&str] = &[
    "claude-fable-5",
    "claude-mythos-5",
    "claude-opus-5",
    "claude-sonnet-5",
    "claude-opus-4-8",
    "claude-opus-4-7",
];
const EFFORT_WITHOUT_XHIGH_FAMILIES: &[&str] = &["claude-opus-4-6", "claude-sonnet-4-6"];
const EFFORT_ONLY_FAMILIES: &[&str] = &["claude-opus-4-5"];

fn bare_model_id(model: &str) -> &str {
    let after_slash = model.rsplit('/').next().unwrap_or(model);
    after_slash
        .strip_prefix("anthropic.")
        .unwrap_or(after_slash)
}

fn in_family(model: &str, family: &str) -> bool {
    model
        .strip_prefix(family)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('-') || rest.starts_with('@'))
}

#[must_use]
pub fn anthropic_effort_support(model: &str) -> EffortSupport {
    let id = bare_model_id(model.trim()).to_ascii_lowercase();
    if FULL_EFFORT_FAMILIES.iter().any(|f| in_family(&id, f)) {
        EffortSupport::Full
    } else if EFFORT_WITHOUT_XHIGH_FAMILIES.iter().any(|f| in_family(&id, f)) {
        EffortSupport::WithoutXhigh
    } else if EFFORT_ONLY_FAMILIES.iter().any(|f| in_family(&id, f)) {
        EffortSupport::EffortOnly
    } else {
        EffortSupport::Unsupported
    }
}

/// Decide the effort fields for `model` from the raw `ARIS_REASONING_EFFORT`
/// value (pure). Depth on current Claude models is `thinking: {type:
/// "adaptive"}` + `output_config.effort`; `budget_tokens` is rejected by them
/// and is never sent. The `OpenAI`-only levels `none` / `minimal` map to `low`
/// (disabling thinking has known failure modes on Opus 5 and is rejected by
/// Fable); `xhigh` maps to `high` on the 4.6 family; Opus 4.5 takes `low` /
/// `medium` / `high` and no adaptive thinking.
#[must_use]
pub fn anthropic_effort_fields(model: &str, raw: Option<&str>) -> EffortDecision {
    let Some(value) = raw.map(str::trim).filter(|v| !v.is_empty()) else {
        return EffortDecision::Unset;
    };
    let requested = value.to_ascii_lowercase();
    let level = match requested.as_str() {
        "none" | "minimal" | "low" => "low",
        "medium" => "medium",
        "high" => "high",
        "xhigh" => "xhigh",
        "max" => "max",
        _ => return EffortDecision::InvalidValue(value.to_string()),
    };
    let support = anthropic_effort_support(model);
    let level = match support {
        EffortSupport::Unsupported => return EffortDecision::UnsupportedModel,
        EffortSupport::WithoutXhigh if level == "xhigh" => "high",
        EffortSupport::EffortOnly if level == "xhigh" || level == "max" => "high",
        EffortSupport::Full | EffortSupport::WithoutXhigh | EffortSupport::EffortOnly => level,
    };
    EffortDecision::Send {
        thinking: (support != EffortSupport::EffortOnly).then(|| json!({ "type": "adaptive" })),
        output_config: json!({ "effort": level }),
    }
}

/// The effort fields for `model` from the process environment, as the two
/// optional request fields. A configured-but-unusable value is reported once
/// per process on stderr and sends nothing.
#[must_use]
pub fn anthropic_effort_from_env(model: &str) -> (Option<Value>, Option<Value>) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static NOTED: AtomicBool = AtomicBool::new(false);
    let raw = std::env::var(REASONING_EFFORT_ENV).ok();
    match anthropic_effort_fields(model, raw.as_deref()) {
        EffortDecision::Send {
            thinking,
            output_config,
        } => (thinking, Some(output_config)),
        EffortDecision::Unset => (None, None),
        EffortDecision::UnsupportedModel => {
            if !NOTED.swap(true, Ordering::SeqCst) {
                eprintln!(
                    "\x1b[2mnote: {REASONING_EFFORT_ENV} is set, but model `{model}` does not take a \
                     reasoning effort on the Anthropic channel; sending the request without it.\x1b[0m"
                );
            }
            (None, None)
        }
        EffortDecision::InvalidValue(value) => {
            if !NOTED.swap(true, Ordering::SeqCst) {
                eprintln!(
                    "\x1b[33mwarning:\x1b[0m {REASONING_EFFORT_ENV}={value} is not an effort level \
                     (low / medium / high / xhigh / max); sending the request without it."
                );
            }
            (None, None)
        }
    }
}

/// Parse an `ARIS_MAX_TOKENS` value: a positive integer, else `None`.
#[must_use]
pub fn parse_max_tokens_override(raw: Option<&str>) -> Option<u32> {
    raw.and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|n| *n > 0)
}

/// Output cap for a STREAMING Anthropic request: the `ARIS_MAX_TOKENS`
/// override when set, else the caller's own default.
#[must_use]
pub fn streaming_max_tokens(caller_default: u32) -> u32 {
    parse_max_tokens_override(std::env::var(MAX_TOKENS_ENV).ok().as_deref())
        .unwrap_or(caller_default)
}

/// Output cap for the NON-streaming recovery request that follows a stream
/// which ended without content: never above the caller's default, because a
/// raised cap is only safe while streaming (a large non-streaming response
/// runs into HTTP timeouts).
#[must_use]
pub fn recovery_max_tokens(streaming_cap: u32, caller_default: u32) -> u32 {
    streaming_cap.min(caller_default)
}

/// The error for a response that spent its whole output cap before producing
/// text or a tool call (`stop_reason: "max_tokens"` with only thinking).
#[must_use]
pub fn output_cap_exhausted_message(max_tokens: u32) -> String {
    format!(
        "the model used the whole output cap ({max_tokens} tokens) before producing an answer. \
         Raise {MAX_TOKENS_ENV} or lower {REASONING_EFFORT_ENV}, then send the message again."
    )
}

/// Append one streamed chunk of tool input to what has accumulated (#444).
///
/// Some relays send a literal `{}` placeholder as the first chunk and the
/// real arguments after it; appending produced `{}{"command":…}`, which fails
/// to parse ("trailing characters at line 1 column 3") and made every tool
/// call unusable. A complete `{}` followed by more JSON text can never be
/// valid, so when the accumulated input is exactly `{}` (whitespace aside) and
/// the new chunk carries non-whitespace, the placeholder is dropped first.
/// Whitespace after a genuine `{}` and ordinary fragmenting are untouched.
pub fn append_tool_input_chunk(accumulated: &mut String, chunk: &str) {
    if accumulated.trim() == "{}" && !chunk.trim().is_empty() {
        accumulated.clear();
    }
    accumulated.push_str(chunk);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effort_support_table_matches_families_not_substrings() {
        for model in [
            "claude-opus-5",
            "claude-opus-5-5",
            "claude-fable-5",
            "claude-fable-5-1",
            "claude-mythos-5-1",
            "claude-sonnet-5",
            "claude-opus-4-8",
            "claude-opus-4-7",
            "claude-opus-4-8-20260115",
            "claude-opus-4-8@20260115",
            "anthropic/claude-opus-5",
            "anthropic.claude-opus-5",
            "CLAUDE-OPUS-5",
        ] {
            assert_eq!(anthropic_effort_support(model), EffortSupport::Full, "{model}");
        }
        for model in ["claude-opus-4-6", "claude-sonnet-4-6", "claude-sonnet-4-6-20260101"] {
            assert_eq!(anthropic_effort_support(model), EffortSupport::WithoutXhigh, "{model}");
        }
        for model in [
            "claude-haiku-4-5-20251001",
            "claude-opus-4-1",
            "claude-opus-4-80",
            "claude-sonnet-4-60",
            "claude-opus-50",
            "deepseek-v4-pro",
            "my-claude-opus-5-proxy-alias",
            "gpt-5.5",
            "",
        ] {
            assert_eq!(anthropic_effort_support(model), EffortSupport::Unsupported, "{model}");
        }
    }

    #[test]
    fn effort_fields_unset_blank_and_invalid_send_nothing() {
        assert_eq!(anthropic_effort_fields("claude-opus-5", None), EffortDecision::Unset);
        assert_eq!(anthropic_effort_fields("claude-opus-5", Some("  ")), EffortDecision::Unset);
        // unset stays unset on the models whose default is "no thinking" too
        assert_eq!(anthropic_effort_fields("claude-opus-4-8", None), EffortDecision::Unset);
        assert_eq!(
            anthropic_effort_fields("claude-opus-5", Some("turbo")),
            EffortDecision::InvalidValue("turbo".to_string())
        );
        assert_eq!(
            anthropic_effort_fields("claude-haiku-4-5-20251001", Some("high")),
            EffortDecision::UnsupportedModel
        );
        assert_eq!(
            anthropic_effort_fields("deepseek-v4-pro", Some("high")),
            EffortDecision::UnsupportedModel
        );
    }

    #[test]
    fn effort_fields_map_levels_and_never_use_budget_tokens() {
        let send = |model: &str, value: &str| match anthropic_effort_fields(model, Some(value)) {
            EffortDecision::Send {
                thinking,
                output_config,
            } => (thinking.unwrap_or(Value::Null), output_config),
            other => panic!("expected Send for {model}/{value}, got {other:?}"),
        };
        for (value, expected) in [
            ("low", "low"),
            ("medium", "medium"),
            ("high", "high"),
            ("xhigh", "xhigh"),
            ("max", "max"),
            ("XHIGH", "xhigh"),
            ("none", "low"),
            ("minimal", "low"),
        ] {
            let (thinking, output_config) = send("claude-opus-5", value);
            assert_eq!(thinking, json!({"type": "adaptive"}), "{value}");
            assert_eq!(output_config, json!({"effort": expected}), "{value}");
            assert!(!thinking.to_string().contains("budget_tokens"));
        }
        // the whole default chain takes the same fields
        for model in ["claude-opus-5", "claude-opus-4-8", "claude-opus-4-7"] {
            assert_eq!(send(model, "xhigh").1, json!({"effort": "xhigh"}), "{model}");
        }
        // Opus 4.5: effort without adaptive thinking, three levels
        for (value, expected) in [("low", "low"), ("high", "high"), ("xhigh", "high"), ("max", "high")] {
            let (thinking, output_config) = send("claude-opus-4-5-20251101", value);
            assert_eq!(thinking, Value::Null, "{value}");
            assert_eq!(output_config, json!({"effort": expected}), "{value}");
        }
        // 4.6 family: xhigh did not exist yet
        assert_eq!(send("claude-sonnet-4-6", "xhigh").1, json!({"effort": "high"}));
        assert_eq!(send("claude-opus-4-6", "max").1, json!({"effort": "max"}));
    }

    #[test]
    fn max_tokens_override_is_a_positive_integer_and_recovery_never_exceeds_the_default() {
        assert_eq!(parse_max_tokens_override(None), None);
        assert_eq!(parse_max_tokens_override(Some("")), None);
        assert_eq!(parse_max_tokens_override(Some("0")), None);
        assert_eq!(parse_max_tokens_override(Some("-5")), None);
        assert_eq!(parse_max_tokens_override(Some("lots")), None);
        assert_eq!(parse_max_tokens_override(Some(" 128000 ")), Some(128_000));
        assert_eq!(parse_max_tokens_override(Some("8000")), Some(8_000));
        // raised cap: streaming keeps it, the non-streaming recovery does not
        assert_eq!(recovery_max_tokens(128_000, 32_000), 32_000);
        // lowered cap applies to both
        assert_eq!(recovery_max_tokens(8_000, 32_000), 8_000);
        assert_eq!(recovery_max_tokens(32_000, 32_000), 32_000);
    }

    #[test]
    fn tool_input_chunks_drop_a_placeholder_but_keep_valid_streams() {
        let feed = |chunks: &[&str]| {
            let mut acc = String::new();
            for chunk in chunks {
                append_tool_input_chunk(&mut acc, chunk);
            }
            acc
        };
        // #444: placeholder, then the real arguments
        assert_eq!(feed(&["{}", r#"{"command":"ls"}"#]), r#"{"command":"ls"}"#);
        assert_eq!(feed(&["{} ", r#"{"command""#, r#":"ls"}"#]), r#"{"command":"ls"}"#);
        // ordinary fragmenting is untouched
        assert_eq!(feed(&[r#"{"a":"#, "1}"]), r#"{"a":1}"#);
        assert_eq!(feed(&["{", "}"]), "{}");
        // a genuine empty-object call, with or without trailing whitespace
        assert_eq!(feed(&["{}"]), "{}");
        assert_eq!(feed(&["{}", " ", "\n"]), "{} \n");
        assert_eq!(feed(&["{}", ""]), "{}");
        for input in [feed(&["{}"]), feed(&["{}", "\n"])] {
            assert!(serde_json::from_str::<Value>(&input).is_ok(), "{input:?}");
        }
        // an object that merely starts with `{}`-like text is not a placeholder
        assert_eq!(feed(&[r#"{"x":{}"#, "}"]), r#"{"x":{}}"#);
    }
}
