use crate::cancel::Cancel;
use crate::llm::{
    CallError, Finish, Generation, Request, Shaping, Speaks, Spelling, Usage, answer_schema,
    finish, knocked, streamed,
};
use anyhow::Result;
use reqwest::{Client, RequestBuilder};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

const URL: &str = "https://api.anthropic.com/v1/messages";
const MODELS_URL: &str = "https://api.anthropic.com/v1/models";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const MAX_TOKENS: u32 = 64_000;

static CAPS: LazyLock<Mutex<HashMap<String, u32>>> = LazyLock::new(Mutex::default);

pub struct Claude {
    client: Client,
    api_key: String,
    model: String,
    max_tokens: u32,
    shaping: Shaping,
}

impl Claude {
    pub async fn new(client: Client, model: String, api_key: String) -> Self {
        let max_tokens = cap(&client, &model, &api_key)
            .await
            .unwrap_or(MAX_TOKENS)
            .min(MAX_TOKENS);

        Self {
            client,
            api_key,
            model,
            max_tokens,
            shaping: Shaping::default(),
        }
    }
}

async fn cap(client: &Client, model: &str, api_key: &str) -> Option<u32> {
    if let Some(known) = CAPS.lock().ok().and_then(|caps| caps.get(model).copied()) {
        return Some(known);
    }

    let found = fetch_cap(client, model, api_key).await?;
    if let Ok(mut caps) = CAPS.lock() {
        caps.insert(model.to_string(), found);
    }

    Some(found)
}

fn listing(client: &Client, model: &str, api_key: &str) -> RequestBuilder {
    client
        .get(format!("{MODELS_URL}/{model}"))
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
}

async fn fetch_cap(client: &Client, model: &str, api_key: &str) -> Option<u32> {
    #[derive(Deserialize)]
    struct Listed {
        max_tokens: Option<u32>,
    }

    listing(client, model, api_key)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<Listed>()
        .await
        .ok()?
        .max_tokens
}

impl Speaks for Claude {
    async fn call(&self, request: Request<'_>) -> Result<Generation, CallError> {
        let outgoing = self
            .client
            .post(URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body(
                &self.model,
                self.max_tokens,
                request.system,
                request.user,
                self.shaping.wanted(),
            ));

        let mut gathered = Gathered::default();
        streamed(outgoing, request.cancel, &self.shaping, |event| {
            gathered.heard(event)
        })
        .await?;

        gathered.into_generation()
    }

    async fn reach(&self, cancel: &Cancel) -> Result<(), CallError> {
        knocked(listing(&self.client, &self.model, &self.api_key), cancel).await
    }
}

fn body(model: &str, max_tokens: u32, system: &str, user: &str, shaped: bool) -> Value {
    let mut asked = json!({
        "model": model,
        "max_tokens": max_tokens,
        "stream": true,
        "system": system,
        "messages": [{ "role": "user", "content": user }],
    });

    if shaped {
        asked["output_config"] = json!({
            "format": {
                "type": "json_schema",
                "schema": answer_schema(Spelling::Closed),
            },
        });
    }

    asked
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Event {
    MessageStart {
        message: Started,
    },
    ContentBlockDelta {
        delta: Delta,
    },
    MessageDelta {
        delta: Ending,
        #[serde(default)]
        usage: Option<Counted>,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct Started {
    #[serde(default)]
    usage: Option<Counted>,
}

#[derive(Debug, Deserialize)]
struct Delta {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Ending {
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    stop_details: Option<StopDetails>,
}

#[derive(Debug, Deserialize)]
struct StopDetails {
    #[serde(default)]
    category: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Counted {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

#[derive(Default)]
struct Gathered {
    text: String,
    stop_reason: Option<String>,
    stop_details: Option<StopDetails>,
    input: u32,
    output: u32,
}

impl Gathered {
    fn heard(&mut self, event: Event) {
        match event {
            Event::MessageStart { message } => {
                if let Some(usage) = message.usage {
                    self.input = usage.input_tokens;
                }
            }
            Event::ContentBlockDelta { delta } => {
                if delta.kind == "text_delta"
                    && let Some(text) = delta.text
                {
                    self.text.push_str(&text);
                }
            }
            Event::MessageDelta { delta, usage } => {
                if delta.stop_reason.is_some() {
                    self.stop_reason = delta.stop_reason;
                }
                if delta.stop_details.is_some() {
                    self.stop_details = delta.stop_details;
                }
                if let Some(usage) = usage {
                    self.output = usage.output_tokens;
                }
            }
            Event::Other => {}
        }
    }

    fn into_generation(self) -> Result<Generation, CallError> {
        let stop_reason = self.stop_reason.unwrap_or_default();

        if stop_reason == "refusal" {
            let category = self
                .stop_details
                .and_then(|details| details.category)
                .unwrap_or_else(|| "refusal".to_string());

            return Err(CallError::Blocked(category));
        }

        finish(
            self.text,
            Finish {
                reason: &stop_reason,
                said: "stop_reason",
                cut: "max_tokens",
                blocked: &[],
            },
            Usage {
                input: self.input,
                output: self.output,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gathered(events: Vec<Value>) -> Result<Generation, CallError> {
        let mut gathered = Gathered::default();
        for event in events {
            gathered.heard(serde_json::from_value::<Event>(event).expect("a valid event"));
        }

        gathered.into_generation()
    }

    fn text_delta(text: &str) -> Value {
        json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "text_delta", "text": text },
        })
    }

    fn ended(stop_reason: &str) -> Value {
        json!({
            "type": "message_delta",
            "delta": { "stop_reason": stop_reason, "stop_sequence": null },
            "usage": { "output_tokens": 34 },
        })
    }

    #[test]
    fn an_answer_split_into_deltas_comes_back_as_one_text() {
        let generation = gathered(vec![
            json!({ "type": "message_start", "message": { "usage": { "input_tokens": 12, "output_tokens": 1 } } }),
            json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } }),
            text_delta("[{\"id\": 0,"),
            text_delta(" \"translation\": \"A\"}]"),
            json!({ "type": "content_block_stop", "index": 0 }),
            ended("end_turn"),
            json!({ "type": "message_stop" }),
        ])
        .unwrap_or_else(|_| panic!("expected a generation"));

        assert_eq!(generation.text, r#"[{"id": 0, "translation": "A"}]"#);
        assert!(!generation.truncated);
    }

    #[test]
    fn what_the_call_cost_is_carried_back_so_the_run_can_bill_it() {
        let generation = gathered(vec![
            json!({ "type": "message_start", "message": { "usage": { "input_tokens": 12, "output_tokens": 1 } } }),
            text_delta("[]"),
            ended("end_turn"),
        ])
        .unwrap_or_else(|_| panic!("expected a generation"));

        assert_eq!(generation.usage.input, 12);
        assert_eq!(
            generation.usage.output, 34,
            "the input count opens the message and the output count closes it, and each \
             provider names these fields its own way, so reading the wrong one bills every call \
             at nothing"
        );
    }

    #[test]
    fn thinking_deltas_and_pings_are_left_out_of_the_answer() {
        let generation = gathered(vec![
            json!({ "type": "ping" }),
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {
                    "type": "thinking_delta",
                    "thinking": "\u{307e}\u{305a}\u{65e5}\u{672c}\u{8a9e}\u{3067}\u{8003}\u{3048}\u{308b}",
                    "text": "\u{307e}\u{305a}\u{65e5}\u{672c}\u{8a9e}\u{3067}\u{8003}\u{3048}\u{308b}",
                },
            }),
            text_delta("[]"),
            ended("end_turn"),
        ])
        .unwrap_or_else(|_| panic!("expected a generation"));

        assert_eq!(
            generation.text, "[]",
            "a model thinking out loud is not answering, and folding that into the answer would \
             hand the reader the model's notes as a translation"
        );
    }

    #[test]
    fn a_refusal_is_reported_with_its_category() {
        let result = gathered(vec![json!({
            "type": "message_delta",
            "delta": {
                "stop_reason": "refusal",
                "stop_details": { "type": "refusal", "category": "cyber" },
            },
        })]);

        assert!(
            matches!(result, Err(CallError::Blocked(reason)) if reason == "cyber"),
            "a refusal is not a failure to retry but a line to leave alone, and saying which \
             category it fell under is what lets the reader judge that"
        );
    }

    #[test]
    fn an_answer_the_model_cut_short_is_reported_as_truncated() {
        let generation = gathered(vec![text_delta("[{\"id\": 0"), ended("max_tokens")])
            .unwrap_or_else(|_| panic!("expected a generation"));

        assert!(
            generation.truncated,
            "an answer that ran out of room is half a batch, and taking it as whole would file \
             the missing lines as done"
        );
    }

    #[test]
    fn a_request_carries_the_prompt_and_none_of_the_knobs_newer_models_refuse() {
        let payload = body("claude-opus-5", MAX_TOKENS, "system", "user", true);

        assert_eq!(payload["model"], json!("claude-opus-5"));
        assert_eq!(payload["max_tokens"], json!(MAX_TOKENS));
        assert_eq!(payload["system"], json!("system"));
        assert_eq!(
            payload["messages"],
            json!([{ "role": "user", "content": "user" }])
        );
        assert_eq!(
            payload["stream"],
            json!(true),
            "a whole answer arrives as one silence with the text at the end, and only a stream \
             lets a connection that went quiet be told apart from a model still writing"
        );
        assert!(
            payload.get("temperature").is_none() && payload.get("thinking").is_none(),
            "a newer model turns a whole request down for carrying a knob it no longer takes, so \
             what the reader set has to be left out of the call rather than sent and ignored"
        );
        assert_eq!(
            payload["output_config"]["format"]["schema"],
            answer_schema(Spelling::Closed)
        );
    }

    #[test]
    fn a_model_that_will_not_take_the_schema_is_still_asked_the_question() {
        let payload = body("claude-opus-5", MAX_TOKENS, "system", "user", false);

        assert!(
            payload.get("output_config").is_none(),
            "an older model answers the contract in the prompt but turns the whole request down \
             for the schema, so dropping the schema is what keeps it translating at all"
        );
        assert_eq!(payload["messages"][0]["content"], json!("user"));
    }
}
