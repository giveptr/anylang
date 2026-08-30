use crate::llm::{
    CallError, Finish, Generation, Request, Shaping, Speaks, Spelling, Usage, answer_schema,
    exchange, finish,
};
use anyhow::Result;
use reqwest::Client;
use reqwest::header::AUTHORIZATION;
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
            url: endpoint(&url),
            api_key,
            model,
            temperature,
            shaping: Shaping::default(),
        }
    }
}

impl Speaks for OpenAiCompatible {
    async fn call(&self, request: Request<'_>) -> Result<Generation, CallError> {
        let outgoing = self
            .client
            .post(&self.url)
            .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
            .json(&body(
                &self.model,
                request.system,
                request.user,
                self.temperature,
                self.shaping.wanted(),
            ));

        let payload: Response = exchange(outgoing, request.cancel, &self.shaping).await?;

        payload.into_generation()
    }
}

fn endpoint(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');

    if trimmed.ends_with("/chat/completions") {
        return trimmed.to_string();
    }

    format!("{trimmed}/chat/completions")
}

fn body(model: &str, system: &str, user: &str, temperature: Option<f32>, shaped: bool) -> Value {
    let mut asked = json!({
        "model": model,
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
    }

    asked
}

#[derive(Debug, Deserialize)]
struct Response {
    #[serde(default)]
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<Counted>,
}

#[derive(Debug, Default, Deserialize)]
struct Counted {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

impl Response {
    fn into_generation(self) -> Result<Generation, CallError> {
        let counted = self.usage.unwrap_or_default();
        let Some(choice) = self.choices.into_iter().next() else {
            return Err(CallError::Retryable(
                "the API returned no choices".to_string(),
            ));
        };

        let finish_reason = choice.finish_reason.unwrap_or_default();
        let text = choice
            .message
            .and_then(|message| message.content)
            .unwrap_or_default();

        finish(
            text,
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

#[derive(Debug, Deserialize)]
struct Choice {
    #[serde(default)]
    message: Option<Message>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Message {
    #[serde(default)]
    content: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_chat_suffix_is_added_exactly_once() {
        for base in [
            "https://api.example.com/v1",
            "https://api.example.com/v1/",
            "https://api.example.com/v1/chat/completions",
            " https://api.example.com/v1 ",
        ] {
            assert_eq!(
                endpoint(base),
                "https://api.example.com/v1/chat/completions"
            );
        }
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

        assert_eq!(payload["temperature"], json!(0.8f32));

        let plain = body("gpt", "system", "user", None, false);
        assert!(plain.get("response_format").is_none());
        assert!(
            plain.get("temperature").is_none(),
            "an endpoint that refuses sampling refuses the whole request over it, so a field \
             left empty has to be left out rather than defaulted"
        );
        assert_eq!(plain["messages"][1]["content"], json!("user"));
    }

    fn parse(value: Value) -> Result<Generation, CallError> {
        serde_json::from_value::<Response>(value)
            .expect("valid response")
            .into_generation()
    }

    #[test]
    fn an_answer_is_taken_from_the_message_the_choice_holds() {
        let generation = parse(json!({
            "choices": [{
                "message": { "role": "assistant", "content": "{\"translations\": []}" },
                "finish_reason": "stop"
            }]
        }))
        .unwrap_or_else(|_| panic!("expected a generation"));

        assert_eq!(generation.text, r#"{"translations": []}"#);
        assert!(
            !generation.truncated,
            "an answer that stopped because it was finished is whole, and marking it short \
             would send the batch round again for nothing"
        );
    }

    #[test]
    fn an_answer_the_model_cut_short_is_reported_as_truncated() {
        let generation = parse(json!({
            "choices": [{ "message": { "content": "[{\"id\"" }, "finish_reason": "length" }]
        }))
        .unwrap_or_else(|_| panic!("expected a generation"));

        assert!(
            generation.truncated,
            "an answer that ran out of room is half a batch, and taking it as whole would file \
             the missing lines as done"
        );
    }

    #[test]
    fn an_answer_the_filter_stopped_is_reported_with_its_reason() {
        let result = parse(json!({
            "choices": [{ "message": { "content": "" }, "finish_reason": "content_filter" }]
        }));
        assert!(
            matches!(result, Err(CallError::Blocked(reason)) if reason == "content_filter"),
            "a filtered answer is a line to leave to the reader, not a fault to retry until the \
             run gives up"
        );
    }
}
