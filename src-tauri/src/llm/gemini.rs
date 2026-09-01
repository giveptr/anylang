use crate::cancel::Cancel;
use crate::llm::{
    CallError, Finish, Generation, Request, Shaping, Speaks, Spelling, Usage, answer_schema,
    finish, knocked, streamed,
};
use anyhow::{Context, Result};
use gcp_auth::{CustomServiceAccount, TokenProvider};
use reqwest::header::AUTHORIZATION;
use reqwest::{Client, RequestBuilder};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path;

const CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";

enum Auth {
    ApiKey(String),
    ServiceAccount {
        account: Box<CustomServiceAccount>,
        project: String,
    },
}

pub struct Gemini {
    client: Client,
    auth: Auth,
    model: String,
    temperature: Option<f32>,
    shaping: Shaping,
}

impl Gemini {
    pub fn api_key(
        client: Client,
        model: String,
        api_key: String,
        temperature: Option<f32>,
    ) -> Self {
        Self {
            client,
            auth: Auth::ApiKey(api_key),
            model,
            temperature,
            shaping: Shaping::default(),
        }
    }

    pub async fn vertex(
        client: Client,
        credentials: &Path,
        model: String,
        temperature: Option<f32>,
    ) -> Result<Self> {
        let raw = tokio::fs::read_to_string(credentials)
            .await
            .with_context(|| format!("reading {}", credentials.display()))?;

        let account = CustomServiceAccount::from_json(&raw)
            .with_context(|| format!("{} is not a service account file", credentials.display()))?;

        let project = account
            .project_id()
            .with_context(|| format!("no project_id in {}", credentials.display()))?
            .to_string();

        Ok(Self {
            client,
            auth: Auth::ServiceAccount {
                account: Box::new(account),
                project,
            },
            model,
            temperature,
            shaping: Shaping::default(),
        })
    }

    fn named(&self) -> String {
        match &self.auth {
            Auth::ApiKey(_) => format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}",
                self.model
            ),
            Auth::ServiceAccount { project, .. } => format!(
                "https://aiplatform.us.rep.googleapis.com/v1/projects/{project}/locations/us/publishers/google/models/{}",
                self.model
            ),
        }
    }

    async fn signed(&self, outgoing: RequestBuilder) -> Result<RequestBuilder, CallError> {
        Ok(match &self.auth {
            Auth::ApiKey(key) => outgoing.header("x-goog-api-key", key),
            Auth::ServiceAccount { account, .. } => {
                let token = account
                    .token(&[CLOUD_PLATFORM_SCOPE])
                    .await
                    .map_err(|error| {
                        CallError::Retryable(format!("could not fetch an access token: {error}"))
                    })?;
                outgoing.header(AUTHORIZATION, format!("Bearer {}", token.as_str()))
            }
        })
    }
}

impl Speaks for Gemini {
    async fn call(&self, request: Request<'_>) -> Result<Generation, CallError> {
        let outgoing = self
            .client
            .post(format!("{}:streamGenerateContent?alt=sse", self.named()))
            .json(&body(
                request.system,
                request.user,
                self.temperature,
                self.shaping.wanted(),
            ));

        let mut gathered = Gathered::default();
        streamed(
            self.signed(outgoing).await?,
            request.cancel,
            &self.shaping,
            |event| gathered.heard(event),
        )
        .await?;

        gathered.into_generation()
    }

    async fn reach(&self, cancel: &Cancel) -> Result<(), CallError> {
        knocked(self.signed(self.client.get(self.named())).await?, cancel).await
    }
}

fn body(system: &str, user: &str, temperature: Option<f32>, shaped: bool) -> Value {
    let mut asked = json!({
        "systemInstruction": { "parts": [{ "text": system }] },
        "contents": [{ "role": "user", "parts": [{ "text": user }] }],
        "generationConfig": {
            "candidateCount": 1,
        },
        "safetySettings": [
            { "category": "HARM_CATEGORY_SEXUALLY_EXPLICIT", "threshold": "BLOCK_NONE" },
            { "category": "HARM_CATEGORY_HATE_SPEECH", "threshold": "BLOCK_NONE" },
            { "category": "HARM_CATEGORY_HARASSMENT", "threshold": "BLOCK_NONE" },
            { "category": "HARM_CATEGORY_DANGEROUS_CONTENT", "threshold": "BLOCK_NONE" },
        ],
    });

    if let Some(temperature) = temperature {
        asked["generationConfig"]["temperature"] = json!(temperature);
    }

    if shaped {
        asked["generationConfig"]["responseMimeType"] = json!("application/json");
        asked["generationConfig"]["responseSchema"] = answer_schema(Spelling::Shouted);
    }

    asked
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Event {
    #[serde(default)]
    candidates: Vec<Candidate>,
    #[serde(default)]
    prompt_feedback: Option<PromptFeedback>,
    #[serde(default)]
    usage_metadata: Option<UsageMetadata>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageMetadata {
    #[serde(default)]
    prompt_token_count: u32,
    #[serde(default)]
    candidates_token_count: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Candidate {
    #[serde(default)]
    content: Option<Content>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Content {
    #[serde(default)]
    parts: Vec<Part>,
}

#[derive(Debug, Deserialize)]
struct Part {
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PromptFeedback {
    #[serde(default)]
    block_reason: Option<String>,
}

#[derive(Default)]
struct Gathered {
    text: String,
    finish_reason: Option<String>,
    block_reason: Option<String>,
    usage: Option<UsageMetadata>,
    answered: bool,
}

impl Gathered {
    fn heard(&mut self, event: Event) {
        if let Some(reason) = event
            .prompt_feedback
            .and_then(|feedback| feedback.block_reason)
        {
            self.block_reason = Some(reason);
        }

        if event.usage_metadata.is_some() {
            self.usage = event.usage_metadata;
        }

        let Some(candidate) = event.candidates.into_iter().next() else {
            return;
        };

        self.answered = true;
        for part in candidate
            .content
            .map(|content| content.parts)
            .unwrap_or_default()
        {
            if let Some(text) = part.text {
                self.text.push_str(&text);
            }
        }
        if candidate.finish_reason.is_some() {
            self.finish_reason = candidate.finish_reason;
        }
    }

    fn into_generation(self) -> Result<Generation, CallError> {
        if let Some(reason) = self.block_reason {
            return Err(CallError::Blocked(reason));
        }

        if !self.answered {
            return Err(CallError::Retryable(
                "the API returned no candidates".to_string(),
            ));
        }

        let counted = self.usage.unwrap_or_default();
        let finish_reason = self.finish_reason.unwrap_or_default();

        finish(
            self.text,
            Finish {
                reason: &finish_reason,
                said: "finishReason",
                cut: "MAX_TOKENS",
                blocked: &[
                    "SAFETY",
                    "PROHIBITED_CONTENT",
                    "BLOCKLIST",
                    "RECITATION",
                    "SPII",
                ],
            },
            Usage {
                input: counted.prompt_token_count,
                output: counted.candidates_token_count,
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

    #[test]
    fn an_answer_split_across_events_and_parts_comes_back_as_one_text() {
        let generation = gathered(vec![
            json!({
                "candidates": [{
                    "content": { "parts": [
                        { "text": "[{\"id\": 0," },
                        { "text": " \"translation\":" }
                    ]}
                }],
                "usageMetadata": { "promptTokenCount": 12, "candidatesTokenCount": 3 }
            }),
            json!({
                "candidates": [{
                    "content": { "parts": [{ "text": " \"A\"}]" }] },
                    "finishReason": "STOP"
                }],
                "usageMetadata": { "promptTokenCount": 12, "candidatesTokenCount": 34 }
            }),
        ])
        .unwrap_or_else(|_| panic!("expected a generation"));

        assert_eq!(generation.text, r#"[{"id": 0, "translation": "A"}]"#);
        assert!(!generation.truncated);
        assert_eq!(
            (generation.usage.input, generation.usage.output),
            (12, 34),
            "the count on each event is the running total, so the last one is the bill"
        );
    }

    #[test]
    fn a_prompt_the_service_blocked_is_reported_with_its_reason() {
        let result = gathered(vec![
            json!({ "promptFeedback": { "blockReason": "SAFETY" } }),
        ]);
        assert!(
            matches!(result, Err(CallError::Blocked(reason)) if reason == "SAFETY"),
            "a blocked prompt comes back with no candidate at all, so reading it as an empty \
             answer would look like the model had nothing to say"
        );
    }

    #[test]
    fn an_answer_the_service_blocked_is_reported_with_its_reason() {
        let result = gathered(vec![json!({
            "candidates": [{ "content": { "parts": [] }, "finishReason": "SAFETY" }]
        })]);
        assert!(
            matches!(result, Err(CallError::Blocked(reason)) if reason == "SAFETY"),
            "a candidate stopped part way holds no lines, and retrying it forever would spend \
             the reader's money on the same refusal"
        );
    }

    #[test]
    fn an_answer_the_model_cut_short_is_reported_as_truncated() {
        let generation = gathered(vec![json!({
            "candidates": [{
                "content": { "parts": [{ "text": "[{\"id\": 0" }] },
                "finishReason": "MAX_TOKENS"
            }]
        })])
        .unwrap_or_else(|_| panic!("expected a generation"));

        assert!(
            generation.truncated,
            "an answer that ran out of room is half a batch, and taking it as whole would file \
             the missing lines as done"
        );
    }

    #[test]
    fn a_stream_that_never_named_a_candidate_is_not_an_answer() {
        let result = gathered(vec![json!({ "usageMetadata": { "promptTokenCount": 12 } })]);
        assert!(
            matches!(result, Err(CallError::Retryable(message)) if message == "the API returned no candidates"),
            "a stream of nothing is a fault at the endpoint worth trying again, not an empty \
             translation to file"
        );
    }
}
