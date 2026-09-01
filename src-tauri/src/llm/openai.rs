use crate::cancel::Cancel;
use crate::llm::{
    CallError, Finish, Generation, Request, Shaping, Speaks, Spelling, Usage, answer_schema,
    finish, knocked, streamed,
};
use anyhow::Result;
use reqwest::header::AUTHORIZATION;
use reqwest::{Client, RequestBuilder};
use serde::Deserialize;
use serde_json::{Value, json};

pub struct OpenAiCompatible {
    client: Client,
    url: String,
    api_key: String,
    model: String,
    temperature: Option<f32>,
    shaping: Shaping,
}

impl OpenAiCompatible {
    pub fn new(
        client: Client,
        url: String,
        model: String,
        api_key: String,
        temperature: Option<f32>,
    ) -> Self {
        Self {
            client,
            url: base(&url),
            api_key,
            model,
            temperature,
            shaping: Shaping::default(),
        }
    }

    fn keyed(&self, outgoing: RequestBuilder) -> RequestBuilder {
        outgoing.header(AUTHORIZATION, format!("Bearer {}", self.api_key))
    }

    fn at(&self, path: &str) -> String {
        format!("{}/{path}", self.url)
    }
}

impl Speaks for OpenAiCompatible {
    async fn call(&self, request: Request<'_>) -> Result<Generation, CallError> {
        let outgoing = self
            .keyed(self.client.post(self.at("chat/completions")))
            .json(&body(
                &self.model,
                request.system,
                request.user,
                self.temperature,
                self.shaping.wanted(),
            ));

        let mut gathered = Gathered::default();
        streamed(outgoing, request.cancel, &self.shaping, |chunk| {
            gathered.heard(chunk)
        })
        .await?;

        gathered.into_generation()
    }

    async fn reach(&self, cancel: &Cancel) -> Result<(), CallError> {
        knocked(self.keyed(self.client.get(self.at("models"))), cancel).await
    }
}

fn base(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

fn body(model: &str, system: &str, user: &str, temperature: Option<f32>, shaped: bool) -> Value {
    let mut asked = json!({
        "model": model,
        "stream": true,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
    });

    if let Some(temperature) = temperature {
        asked["temperature"] = json!(temperature);
    }

    if shaped {
        asked["response_format"] = json!({
            "type": "json_schema",
            "json_schema": {
                "name": "translations",
                "schema": answer_schema(Spelling::Plain),
            },
        });
        asked["stream_options"] = json!({ "include_usage": true });
    }

    asked
}

#[derive(Debug, Deserialize)]
struct Chunk {
    #[serde(default)]
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<Counted>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    #[serde(default)]
    delta: Option<Delta>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Counted {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

#[derive(Default)]
struct Gathered {
    text: String,
    finish_reason: Option<String>,
    usage: Option<Counted>,
    answered: bool,
}

impl Gathered {
    fn heard(&mut self, chunk: Chunk) {
        if let Some(choice) = chunk.choices.into_iter().next() {
            self.answered = true;
            if let Some(content) = choice.delta.and_then(|delta| delta.content) {
                self.text.push_str(&content);
            }
            if choice.finish_reason.is_some() {
                self.finish_reason = choice.finish_reason;
            }
        }

        if chunk.usage.is_some() {
            self.usage = chunk.usage;
        }
    }

    fn into_generation(self) -> Result<Generation, CallError> {
        if !self.answered {
            return Err(CallError::Retryable(
                "the API returned no choices".to_string(),
            ));
        }

        let counted = self.usage.unwrap_or_default();
        let finish_reason = self.finish_reason.unwrap_or_default();

        finish(
            self.text,
            Finish {
                reason: &finish_reason,
                said: "finish_reason",
                cut: "length",
                blocked: &["content_filter"],
            },
            Usage {
                input: counted.prompt_tokens,
                output: counted.completion_tokens,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn what_is_kept_is_the_base_every_path_hangs_off() {
        for pasted in [
            "https://api.example.com/v1",
            "https://api.example.com/v1/",
            " https://api.example.com/v1 ",
        ] {
            assert_eq!(
                base(pasted),
                "https://api.example.com/v1",
                "a stray space or a trailing slash is how one address looks coming off a \
                 clipboard, and both hang their paths in the same place"
            );
        }

        assert_eq!(
            base("https://api.example.com/v1/chat/completions"),
            "https://api.example.com/v1/chat/completions",
            "a URL that is not the base is a different address, and rewriting it quietly would \
             leave the reader looking at a setting nobody typed"
        );
    }

    #[test]
    fn the_endpoint_is_told_the_shape_the_answer_must_take() {
        let payload = body("gpt", "system", "user", Some(0.8), true);

        assert_eq!(
            payload["response_format"]["json_schema"]["schema"],
            answer_schema(Spelling::Plain),
            "an endpoint left to guess hands back the source key it was given, and a batch \
             answered under the wrong key is fifty lines paid for and thrown away"
        );
        assert_eq!(
            payload["stream_options"],
            json!({ "include_usage": true }),
            "a stream carries no usage unless asked, and a run that cannot say what it spent \
             leaves the reader guessing at the bill"
        );

        assert_eq!(payload["temperature"], json!(0.8f32));

        let plain = body("gpt", "system", "user", None, false);
        assert!(plain.get("response_format").is_none());
        assert!(
            plain.get("stream_options").is_none(),
            "an endpoint that turned the request down as malformed gets the bare form next, \
             with nothing on it that a strict server might refuse"
        );
        assert!(
            plain.get("temperature").is_none(),
            "an endpoint that refuses sampling refuses the whole request over it, so a field \
             left empty has to be left out rather than defaulted"
        );
        assert_eq!(plain["messages"][1]["content"], json!("user"));
    }

    #[test]
    fn every_request_asks_for_the_answer_as_a_stream() {
        for shaped in [true, false] {
            assert_eq!(
                body("gpt", "system", "user", None, shaped)["stream"],
                json!(true),
                "a whole answer arrives as one silence with the text at the end, and only a \
                 stream lets a proxy that went quiet be told apart from a model still writing"
            );
        }
    }

    fn gathered(chunks: Vec<Value>) -> Result<Generation, CallError> {
        let mut gathered = Gathered::default();
        for chunk in chunks {
            gathered.heard(serde_json::from_value::<Chunk>(chunk).expect("a valid chunk"));
        }

        gathered.into_generation()
    }

    #[test]
    fn an_answer_arriving_in_pieces_comes_back_as_one_text() {
        let generation = gathered(vec![
            json!({ "choices": [{ "delta": { "role": "assistant", "content": "" }, "finish_reason": null }] }),
            json!({ "choices": [{ "delta": { "content": "{\"translations\":" }, "finish_reason": null }] }),
            json!({ "choices": [{ "delta": { "content": " []}" }, "finish_reason": null }] }),
            json!({ "choices": [{ "delta": {}, "finish_reason": "stop" }] }),
        ])
        .unwrap_or_else(|_| panic!("expected a generation"));

        assert_eq!(generation.text, r#"{"translations": []}"#);
        assert!(
            !generation.truncated,
            "an answer that stopped because it was finished is whole, and marking it short \
             would send the batch round again for nothing"
        );
    }

    #[test]
    fn what_the_call_cost_arrives_after_the_last_choice() {
        let generation = gathered(vec![
            json!({ "choices": [{ "delta": { "content": "[]" }, "finish_reason": "stop" }] }),
            json!({ "choices": [], "usage": { "prompt_tokens": 12, "completion_tokens": 34 } }),
        ])
        .unwrap_or_else(|_| panic!("expected a generation"));

        assert_eq!(generation.usage.input, 12);
        assert_eq!(
            generation.usage.output, 34,
            "the usage rides in a trailing chunk with no choices, and a reader that stopped \
             listening at the finish reason would bill every call at nothing"
        );
    }

    #[test]
    fn an_answer_the_model_cut_short_is_reported_as_truncated() {
        let generation = gathered(vec![json!({
            "choices": [{ "delta": { "content": "[{\"id\"" }, "finish_reason": "length" }]
        })])
        .unwrap_or_else(|_| panic!("expected a generation"));

        assert!(
            generation.truncated,
            "an answer that ran out of room is half a batch, and taking it as whole would file \
             the missing lines as done"
        );
    }

    #[test]
    fn an_answer_the_filter_stopped_is_reported_with_its_reason() {
        let result = gathered(vec![json!({
            "choices": [{ "delta": { "content": "" }, "finish_reason": "content_filter" }]
        })]);
        assert!(
            matches!(result, Err(CallError::Blocked(reason)) if reason == "content_filter"),
            "a filtered answer is a line to leave to the reader, not a fault to retry until the \
             run gives up"
        );
    }

    #[test]
    fn a_stream_that_never_named_a_choice_is_not_an_answer() {
        let result = gathered(vec![json!({ "choices": [] })]);
        assert!(
            matches!(result, Err(CallError::Retryable(message)) if message == "the API returned no choices"),
            "a stream of nothing is a fault at the endpoint worth trying again, not an empty \
             translation to file"
        );
    }
}
