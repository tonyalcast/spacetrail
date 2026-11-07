//! Calls to the user's chosen LLM provider. Used only when the user presses
//! "Explain"; nothing else in spacetrail touches the network.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::settings::{Provider, Settings};

const ANTHROPIC_API: &str = "https://api.anthropic.com/v1";
const ANTHROPIC_VERSION: &str = "2026-10-21";
const OPENAI_API: &str = "https://api.openai.com/v1";

/// Models that accept Anthropic's server-side refusal fallback.
const FALLBACK_MODELS: &[&str] = &[
    "claude-fable-5-1",
    "claude-opus-5-5",
    "claude-opus-5",
    "claude-sonnet-5-5",
];

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        // Never follow redirects: `x-api-key` is not stripped on a cross-host redirect.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())
}

fn base_url(p: Provider, s: &Settings) -> String {
    match p {
        Provider::Anthropic => ANTHROPIC_API.into(),
        Provider::Openai => OPENAI_API.into(),
        Provider::Compatible => s.compatible_base_url.trim_end_matches('/').to_owned(),
    }
}

/// A model offered by a provider.
#[derive(Debug, Serialize)]
pub(crate) struct ModelInfo {
    pub(crate) id: String,
    pub(crate) name: String,
}

async fn send(req: reqwest::RequestBuilder) -> Result<Value, String> {
    let resp = req
        .send()
        .await
        .map_err(|e| format!("Could not reach the provider: {e}"))?;
    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("Unexpected response ({status}): {e}"))?;
    if status.is_success() {
        Ok(body)
    } else {
        let msg = body
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("request failed");
        Err(match status.as_u16() {
            401 | 403 => format!("The API key was rejected ({status}): {msg}"),
            429 => format!("Rate limited by the provider: {msg}"),
            _ => format!("{status}: {msg}"),
        })
    }
}

/// Lists the models the key can use.
pub(crate) async fn list_models(
    p: Provider,
    s: &Settings,
    key: Option<&str>,
) -> Result<Vec<ModelInfo>, String> {
    let url = format!("{}/models", base_url(p, s));
    let req = match p {
        Provider::Anthropic => client()?
            .get(format!("{url}?limit=100"))
            .header("x-api-key", key.ok_or("No API key saved")?)
            .header("anthropic-version", ANTHROPIC_VERSION),
        Provider::Openai | Provider::Compatible => {
            let mut r = client()?.get(url);
            if let Some(k) = key {
                r = r.bearer_auth(k);
            } else if p == Provider::Openai {
                return Err("No API key saved".into());
            }
            r
        }
    };
    let body = send(req).await?;
    let mut models: Vec<ModelInfo> = body
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let id = m.get("id")?.as_str()?.to_owned();
            let name = m
                .get("display_name")
                .and_then(Value::as_str)
                .unwrap_or(&id)
                .to_owned();
            Some(ModelInfo { id, name })
        })
        .collect();
    if p != Provider::Anthropic {
        models.sort_by(|a, b| a.id.cmp(&b.id));
    }
    Ok(models)
}

/// The structured answer we ask every model for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Answer {
    /// What the item is, in one or two sentences.
    pub(crate) what: String,
    /// Which app, tool or process created it, and why.
    pub(crate) origin: String,
    /// "safe", "caution" or "keep".
    pub(crate) verdict: String,
    /// What happens after deleting it.
    pub(crate) impact: String,
    /// The proper way to remove it.
    pub(crate) how_to_remove: String,
    /// "high", "medium" or "low".
    pub(crate) confidence: String,
}

fn schema() -> Value {
    let s = |d: &str| json!({ "type": "string", "description": d });
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["what", "origin", "verdict", "impact", "how_to_remove", "confidence"],
        "properties": {
            "what": s("What this file or folder is, in one or two sentences."),
            "origin": s("Which app, tool, system component or user action created it, and why it exists."),
            "verdict": { "type": "string", "enum": ["safe", "caution", "keep"], "description": "safe = deleting causes no harm or regenerates automatically; caution = regenerable but costly, or may hold something wanted; keep = personal data, needed by the system or an app, or not recoverable." },
            "impact": s("Concretely what happens after deleting it: what breaks, what is rebuilt or re-downloaded, what is lost forever."),
            "how_to_remove": s("The proper way to remove it (inside the app, a tool's own cleanup command, or moving it to the Trash), or why not to."),
            "confidence": { "type": "string", "enum": ["high", "medium", "low"] }
        }
    })
}

fn system_prompt(language: &str) -> String {
    format!(
        "You are a macOS storage expert built into spacetrail, a disk-space app. The user is deciding whether to delete a file or folder and needs to understand it first. \
You only get metadata (path, size, dates, download origin, names of a few items inside); you never see file contents. \
Identify which app, developer tool, system component or user action created the item, what it is for, and what deleting it would change. \
Be concrete and honest: if the evidence is thin, say what it most likely is, lower the confidence and say what to check. \
Never recommend deleting anything inside /System, and treat user documents, photos, databases and app libraries as keep unless clearly disposable. \
Write all text fields in {language}. Reply with the JSON object only."
    )
}

/// Asks the active provider about `facts` (already JSON). Returns the answer and the model used.
pub(crate) async fn explain(
    p: Provider,
    s: &Settings,
    key: Option<&str>,
    facts: &str,
) -> Result<(Answer, String), String> {
    let model = s.model(p).trim().to_owned();
    if model.is_empty() {
        return Err(format!("Choose a {} model in Settings first.", p.label()));
    }
    let system = system_prompt(&s.language);
    let user = format!("Explain this item so I can decide whether to delete it:\n{facts}");
    let text = match p {
        Provider::Anthropic => {
            let mut body = json!({
                "model": model,
                "max_tokens": 16000,
                "system": system,
                "messages": [{ "role": "user", "content": user }],
                "output_config": { "effort": "low", "format": { "type": "json_schema", "schema": schema() } }
            });
            let mut req = client()?
                .post(format!("{ANTHROPIC_API}/messages"))
                .header("x-api-key", key.ok_or("No Anthropic API key saved")?)
                .header("anthropic-version", ANTHROPIC_VERSION);
            if FALLBACK_MODELS.contains(&model.as_str()) {
                body["fallbacks"] = json!("default");
                req = req.header("anthropic-beta", "server-side-fallback-2026-07-01");
            }
            let resp = send(req.json(&body)).await?;
            if resp.get("stop_reason").and_then(Value::as_str) == Some("refusal") {
                return Err("The model declined to answer this request.".into());
            }
            resp.get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<String>()
        }
        Provider::Openai | Provider::Compatible => {
            let body = json!({
                "model": model,
                "messages": [
                    { "role": "system", "content": format!("{system}\nJSON schema:\n{}", schema()) },
                    { "role": "user", "content": user }
                ],
                "response_format": { "type": "json_object" }
            });
            let mut req = client()?
                .post(format!("{}/chat/completions", base_url(p, s)))
                .json(&body);
            if let Some(k) = key {
                req = req.bearer_auth(k);
            } else if p == Provider::Openai {
                return Err("No OpenAI API key saved".into());
            }
            let resp = send(req).await?;
            resp.pointer("/choices/0/message/content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        }
    };
    Ok((parse_answer(&text)?, model))
}

/// Extracts the JSON object from a model reply (tolerates code fences or prose around it).
fn parse_answer(text: &str) -> Result<Answer, String> {
    let start = text.find('{');
    let end = text.rfind('}');
    let json = match (start, end) {
        (Some(a), Some(b)) if b > a => &text[a..=b],
        _ => {
            return Err(format!(
                "The model did not return an answer in the expected format:\n{text}"
            ));
        }
    };
    serde_json::from_str(json)
        .map_err(|e| format!("Could not read the model's answer ({e}):\n{text}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fenced_json() {
        let a = parse_answer("```json\n{\"what\":\"w\",\"origin\":\"o\",\"verdict\":\"safe\",\"impact\":\"i\",\"how_to_remove\":\"h\",\"confidence\":\"high\"}\n```").unwrap();
        assert_eq!(a.verdict, "safe");
        assert!(parse_answer("no json here").is_err());
    }
}
