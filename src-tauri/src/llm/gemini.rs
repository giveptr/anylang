use crate::llm::{CallError, Finish, Generation, Request, Speaks, Usage, exchange, finish};
use anyhow::{Context, Result};
use gcp_auth::{CustomServiceAccount, TokenProvider};
use reqwest::Client;
use reqwest::header::AUTHORIZATION;
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
    temperature: f32,
}

impl Gemini {
    pub fn api_key(client: Client, model: String, api_key: String, temperature: f32) -> Self {
        Self {
            client,
            auth: Auth::ApiKey(api_key),
            model,
            temperature,
        }
    }

    pub async fn vertex(
        client: Client,
        credentials: &Path,
        model: String,
        temperature: f32,
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
        })
    }

    fn endpoint(&self) -> String {
        match &self.auth {
            Auth::ApiKey(_) => format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
                self.model
            ),
            Auth::ServiceAccount { project, .. } => format!(
                "https://aiplatform.us.rep.googleapis.com/v1/projects/{project}/locations/us/publishers/google/models/{}:generateContent",
                self.model
            ),
        }
    }
}

impl Speaks for Gemini {
    async fn call(&self, request: Request<'_>) -> Result<Generation, CallError> {
        let mut outgoing = self.client.post(self.endpoint()).json(&body(
            request.system,
            request.user,
            self.temperature,
        ));

        match &self.auth {
            Auth::ApiKey(key) => outgoing = outgoing.header("x-goog-api-key", key),
            Auth::ServiceAccount { account, .. } => {
                let token = account
                    .token(&[CLOUD_PLATFORM_SCOPE])
                    .await
                    .map_err(|error| {
                        CallError::Retryable(format!("could not fetch an access token: {error}"))
                    })?;
                outgoing = outgoing.header(AUTHORIZATION, format!("Bearer {}", token.as_str()));
            }
        }

        let payload: Response = exchange(outgoing, request.cancel).await?;

        payload.into_generation()
    }
}

fn body(system: &str, user: &str, temperature: f32) -> Value {
    json!({
        "systemInstruction": { "parts": [{ "text": system }] },
        "contents": [{ "role": "user", "parts": [{ "text": user }] }],
        "generationConfig": {
            "temperature": temperature,
            "candidateCount": 1,
            "responseMimeType": "application/json",
            "responseSchema": {
                "type": "ARRAY",
                "items": {
                    "type": "OBJECT",
                    "properties": {
                        "id": { "type": "INTEGER" },
                        "translation": { "type": "STRING" },
                    },
                    "required": ["id", "translation"],
                },
            },
        },
        "safetySettings": [
            { "category": "HARM_CATEGORY_SEXUALLY_EXPLICIT", "threshold": "BLOCK_NONE" },
            { "category": "HARM_CATEGORY_HATE_SPEECH", "threshold": "BLOCK_NONE" },
            { "category": "HARM_CATEGORY_HARASSMENT", "threshold": "BLOCK_NONE" },
            { "category": "HARM_CATEGORY_DANGEROUS_CONTENT", "threshold": "BLOCK_NONE" },
        ],
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Response {
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

impl Response {
    fn into_generation(self) -> Result<Generation, CallError> {
        let counted = self.usage_metadata.unwrap_or_default();
        if let Some(feedback) = &self.prompt_feedback
            && let Some(reason) = &feedback.block_reason
        {
            return Err(CallError::Blocked(reason.clone()));
        }

        let Some(candidate) = self.candidates.into_iter().next() else {
            return Err(CallError::Retryable(
                "the API returned no candidates".to_string(),
            ));
        };

        let finish_reason = candidate.finish_reason.unwrap_or_default();
        let text = candidate
            .content
            .map(|content| {
                content
                    .parts
                    .into_iter()
                    .filter_map(|part| part.text)
                    .collect::<String>()
            })
            .unwrap_or_default();

        finish(
            text,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(value: Value) -> Result<Generation, CallError> {
        serde_json::from_value::<Response>(value)
            .expect("valid response")
            .into_generation()
    }

    #[test]
    fn an_answer_split_into_parts_comes_back_as_one_text() {
        let generation = parse(json!({
            "candidates": [{
                "content": { "parts": [
                    { "text": "[{\"id\": 0," },
                    { "text": " \"translation\": \"A\"}]" }
                ]},
                "finishReason": "STOP"
            }]
        }))
        .unwrap_or_else(|_| panic!("expected a generation"));

        assert_eq!(generation.text, r#"[{"id": 0, "translation": "A"}]"#);
        assert!(!generation.truncated);
    }

    #[test]
    fn a_prompt_the_service_blocked_is_reported_with_its_reason() {
        let result = parse(json!({ "promptFeedback": { "blockReason": "SAFETY" } }));
        assert!(
            matches!(result, Err(CallError::Blocked(reason)) if reason == "SAFETY"),
            "a blocked prompt comes back with no candidate at all, so reading it as an empty \
             answer would look like the model had nothing to say"
        );
    }

    #[test]
    fn an_answer_the_service_blocked_is_reported_with_its_reason() {
        let result = parse(json!({
            "candidates": [{ "content": { "parts": [] }, "finishReason": "SAFETY" }]
        }));
        assert!(
            matches!(result, Err(CallError::Blocked(reason)) if reason == "SAFETY"),
            "a candidate stopped part way holds no lines, and retrying it forever would spend \
             the reader's money on the same refusal"
        );
    }

    #[test]
    fn an_answer_the_model_cut_short_is_reported_as_truncated() {
        let generation = parse(json!({
            "candidates": [{
                "content": { "parts": [{ "text": "[{\"id\": 0" }] },
                "finishReason": "MAX_TOKENS"
            }]
        }))
        .unwrap_or_else(|_| panic!("expected a generation"));

        assert!(
            generation.truncated,
            "an answer that ran out of room is half a batch, and taking it as whole would file \
             the missing lines as done"
        );
    }
}
