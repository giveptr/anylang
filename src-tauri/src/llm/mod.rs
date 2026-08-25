pub mod claude;
pub mod gemini;
pub mod openai;

use crate::cancel::Cancel;
use crate::llm::claude::Claude;
use crate::llm::gemini::Gemini;
use crate::llm::openai::OpenAiCompatible;
use crate::progress::{Progress, Source};
use crate::settings::Settings;
use crate::tuning::Tuning;
use anyhow::{Context, Result, anyhow, bail};
use futures::future::{BoxFuture, FutureExt};
use reqwest::{Client, RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::fmt;
use std::future::Future;
use std::path::Path;
use tokio::time::{self, MissedTickBehavior};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    #[default]
    Gemini,
    Vertex,
    Claude,
    Compatible,
}

pub struct Request<'a> {
    pub system: &'a str,
    pub user: &'a str,
    pub cancel: &'a Cancel,
}

pub struct Generation {
    pub text: String,
    pub truncated: bool,
    pub usage: Usage,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Usage {
    pub input: u32,
    pub output: u32,
}

impl Usage {
    pub fn add(&mut self, other: Usage) {
        self.input += other.input;
        self.output += other.output;
    }

    pub fn total(&self) -> u32 {
        self.input + self.output
    }

    pub fn told(&self) -> String {
        format!(
            "{} tokens ({} in, {} out)",
            self.total(),
            self.input,
            self.output
        )
    }
}

#[derive(Debug)]
pub enum CallError {
    Retryable(String),
    Fatal(anyhow::Error),
    Blocked(String),
    Stopped,
}

impl fmt::Display for CallError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Retryable(message) | Self::Blocked(message) => out.write_str(message),
            Self::Fatal(error) => write!(out, "{error:#}"),
            Self::Stopped => out.write_str("stopped"),
        }
    }
}

pub trait Model: Send + Sync {
    fn generate<'a>(&'a self, request: Request<'a>)
    -> BoxFuture<'a, Result<Generation, CallError>>;
}

pub trait Speaks: Send + Sync {
    fn call(
        &self,
        request: Request<'_>,
    ) -> impl Future<Output = Result<Generation, CallError>> + Send;
}

impl<T: Speaks> Model for T {
    fn generate<'a>(
        &'a self,
        request: Request<'a>,
    ) -> BoxFuture<'a, Result<Generation, CallError>> {
        self.call(request).boxed()
    }
}

struct Finish<'a> {
    reason: &'a str,
    said: &'a str,
    cut: &'a str,
    blocked: &'a [&'a str],
}

fn finish(text: String, at: Finish<'_>, usage: Usage) -> Result<Generation, CallError> {
    if text.trim().is_empty() {
        if at.blocked.contains(&at.reason) {
            return Err(CallError::Blocked(at.reason.to_string()));
        }

        return Err(CallError::Retryable(format!(
            "empty output ({}={})",
            at.said, at.reason
        )));
    }

    Ok(Generation {
        truncated: at.reason == at.cut,
        text,
        usage,
    })
}

fn http_client(tuning: &Tuning) -> Result<Client> {
    Client::builder()
        .timeout(tuning.request_timeout)
        .build()
        .context("building the HTTP client")
}

fn needed(value: &str, what: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{what} is empty. Fill it in under Settings.");
    }

    Ok(value.to_string())
}

pub async fn build(settings: &Settings, tuning: &Tuning) -> Result<Box<dyn Model>> {
    let client = http_client(tuning)?;

    match settings.using {
        Provider::Gemini => {
            let it = &settings.gemini;
            Ok(Box::new(Gemini::api_key(
                client,
                needed(&it.model, "The model name")?,
                needed(&it.api_key, "The Gemini API key")?,
                it.temperature.heated(),
            )))
        }
        Provider::Vertex => {
            let it = &settings.vertex;
            Ok(Box::new(
                Gemini::vertex(
                    client,
                    Path::new(&needed(&it.credentials, "The Vertex service account file")?),
                    needed(&it.model, "The model name")?,
                    it.temperature.heated(),
                )
                .await?,
            ))
        }
        Provider::Claude => {
            let it = &settings.claude;
            Ok(Box::new(
                Claude::new(
                    client,
                    needed(&it.model, "The model name")?,
                    needed(&it.api_key, "The Claude API key")?,
                )
                .await,
            ))
        }
        Provider::Compatible => {
            let it = &settings.compatible;
            Ok(Box::new(OpenAiCompatible::new(
                client,
                needed(&it.base_url, "The endpoint")?,
                needed(&it.model, "The model name")?,
                needed(&it.api_key, "The API key")?,
                it.temperature.heated(),
            )))
        }
    }
}

async fn exchange<T: DeserializeOwned>(
    outgoing: RequestBuilder,
    cancel: &Cancel,
) -> Result<T, CallError> {
    let response = until_stopped(cancel, outgoing.send())
        .await?
        .map_err(transport_error)?;

    let status = response.status();
    if !status.is_success() {
        let body = until_stopped(cancel, response.text())
            .await?
            .unwrap_or_default();
        return Err(classify_status(status, &body));
    }

    until_stopped(cancel, response.json())
        .await?
        .map_err(|error| CallError::Retryable(format!("response body was not valid JSON: {error}")))
}

async fn until_stopped<F: Future>(cancel: &Cancel, work: F) -> Result<F::Output, CallError> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(CallError::Stopped),
        done = work => Ok(done),
    }
}

pub async fn generate_with_retry(
    model: &dyn Model,
    request: Request<'_>,
    tuning: &Tuning,
    progress: &dyn Progress,
) -> Result<Generation, CallError> {
    let cancel = request.cancel;
    let mut ticker = time::interval(tuning.retry_delay);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let mut attempt = 0;
    let mut told: Option<String> = None;
    loop {
        until_stopped(cancel, ticker.tick()).await?;

        let answer = model.generate(Request {
            system: request.system,
            user: request.user,
            cancel,
        });

        match answer.await {
            Ok(generation) => return Ok(generation),
            Err(CallError::Retryable(message)) => {
                if attempt >= tuning.max_retries {
                    return Err(CallError::Retryable(format!(
                        "gave up after {} retries: {message}",
                        tuning.max_retries
                    )));
                }

                if told.as_deref() != Some(&message) {
                    progress.warn(Source::Translate, &format!("retrying: {message}"));
                    told = Some(message);
                }
                attempt += 1;
            }
            Err(other) => return Err(other),
        }
    }
}

fn overflows(status: StatusCode, body: &str) -> bool {
    if status == StatusCode::PAYLOAD_TOO_LARGE {
        return true;
    }

    if !matches!(
        status,
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY
    ) {
        return false;
    }

    let lowered = body.to_lowercase();
    [
        "context length",
        "context_length",
        "context limit",
        "context size",
        "too many tokens",
        "token limit",
        "max_tokens",
        "too long",
    ]
    .iter()
    .any(|mark| lowered.contains(mark))
}

fn classify_status(status: StatusCode, body: &str) -> CallError {
    let snippet: String = body.trim().chars().take(300).collect();

    if overflows(status, body) {
        return CallError::Blocked(format!("HTTP {status} - {snippet}"));
    }

    match status {
        StatusCode::TOO_MANY_REQUESTS
        | StatusCode::REQUEST_TIMEOUT
        | StatusCode::INTERNAL_SERVER_ERROR
        | StatusCode::BAD_GATEWAY
        | StatusCode::SERVICE_UNAVAILABLE
        | StatusCode::GATEWAY_TIMEOUT => CallError::Retryable(format!("HTTP {status} - {snippet}")),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => CallError::Fatal(anyhow!(
            "HTTP {status} - authentication failed, check the API key\n{snippet}"
        )),
        StatusCode::NOT_FOUND => CallError::Fatal(anyhow!(
            "HTTP {status} - model or endpoint not found, check the model name\n{snippet}"
        )),
        _ => CallError::Retryable(format!("HTTP {status} - {snippet}")),
    }
}

fn transport_error(error: reqwest::Error) -> CallError {
    if !error.is_builder() && (error.is_timeout() || error.is_connect() || error.is_request()) {
        CallError::Retryable(format!("transport error: {error}"))
    } else {
        CallError::Fatal(anyhow!("request failed: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::{Heard, Quiet};
    use std::future;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct Flaky {
        failures_left: AtomicU32,
        fatal: bool,
    }

    impl Speaks for Flaky {
        async fn call(&self, _request: Request<'_>) -> Result<Generation, CallError> {
            let left = self.failures_left.load(Ordering::Relaxed);
            if left > 0 {
                self.failures_left.store(left - 1, Ordering::Relaxed);
                let error = if self.fatal {
                    CallError::Fatal(anyhow!("bad key"))
                } else {
                    CallError::Retryable("busy".to_string())
                };
                return Err(error);
            }

            Ok(Generation {
                text: "ok".to_string(),
                truncated: false,
                usage: Usage::default(),
            })
        }
    }

    struct Grumbling {
        failures_left: AtomicU32,
    }

    impl Speaks for Grumbling {
        async fn call(&self, _request: Request<'_>) -> Result<Generation, CallError> {
            let left = self.failures_left.load(Ordering::Relaxed);
            if left > 0 {
                self.failures_left.store(left - 1, Ordering::Relaxed);
                return Err(CallError::Retryable(format!("wall {left}")));
            }

            Ok(Generation {
                text: "ok".to_string(),
                truncated: false,
                usage: Usage::default(),
            })
        }
    }

    fn request<'a>(cancel: &'a Cancel) -> Request<'a> {
        Request {
            system: "system",
            user: "user",
            cancel,
        }
    }

    #[test]
    fn only_a_wall_every_request_would_hit_stops_the_whole_run() {
        let sorted = |status: StatusCode| classify_status(status, "");

        assert!(
            matches!(sorted(StatusCode::UNAUTHORIZED), CallError::Fatal(_)),
            "a wrong key fails every later request too, so there is nothing to keep going for"
        );
        assert!(matches!(sorted(StatusCode::FORBIDDEN), CallError::Fatal(_)));
        assert!(matches!(sorted(StatusCode::NOT_FOUND), CallError::Fatal(_)));

        assert!(
            matches!(sorted(StatusCode::BAD_REQUEST), CallError::Retryable(_)),
            "one odd request may not cost the reader the rest of the run"
        );
        assert!(matches!(
            sorted(StatusCode::CONFLICT),
            CallError::Retryable(_)
        ));
    }

    #[tokio::test]
    async fn a_transient_failure_is_retried_until_it_works() {
        let model = Flaky {
            failures_left: AtomicU32::new(3),
            fatal: false,
        };

        let cancel = Cancel::default();
        let generation = generate_with_retry(&model, request(&cancel), &Tuning::instant(), &Quiet)
            .await
            .unwrap_or_else(|_| panic!("should have recovered"));

        assert_eq!(
            generation.text, "ok",
            "a service that wobbles is the normal case over a long run, and giving up on the \
             first stumble would lose a batch that was one retry from working"
        );
    }

    #[tokio::test]
    async fn a_stall_is_reported_once_until_the_failure_changes_its_words() {
        let cancel = Cancel::default();

        let smooth = Flaky {
            failures_left: AtomicU32::new(0),
            fatal: false,
        };
        let quiet = Heard::default();
        generate_with_retry(&smooth, request(&cancel), &Tuning::instant(), &quiet)
            .await
            .unwrap_or_else(|_| panic!("expected an answer"));

        assert!(
            quiet.warnings().is_empty(),
            "an answer that came first try has nothing to report"
        );

        let bumpy = Flaky {
            failures_left: AtomicU32::new(2),
            fatal: false,
        };
        let heard = Heard::default();
        generate_with_retry(&bumpy, request(&cancel), &Tuning::instant(), &heard)
            .await
            .unwrap_or_else(|_| panic!("should have recovered"));

        assert_eq!(
            heard.warnings(),
            ["retrying: busy"],
            "a second identical failure adds nothing the first line did not say, and twenty \
             calls waiting out one outage would fill the log with copies of it"
        );

        let wordy = Grumbling {
            failures_left: AtomicU32::new(2),
        };
        let heard = Heard::default();
        generate_with_retry(&wordy, request(&cancel), &Tuning::instant(), &heard)
            .await
            .unwrap_or_else(|_| panic!("should have recovered"));

        assert_eq!(
            heard.warnings(),
            ["retrying: wall 2", "retrying: wall 1"],
            "a failure that changed its words is news again, or the reader watches a silent \
             log while the reason their run is stuck moves on without them"
        );
    }

    #[tokio::test]
    async fn a_call_that_keeps_failing_stops_when_the_retries_run_out() {
        let tuning = Tuning {
            max_retries: 2,
            ..Tuning::instant()
        };
        let model = Flaky {
            failures_left: AtomicU32::new(99),
            fatal: false,
        };

        let cancel = Cancel::default();
        let result = generate_with_retry(&model, request(&cancel), &tuning, &Quiet).await;

        assert!(matches!(
            result,
            Err(CallError::Retryable(message)) if message == "gave up after 2 retries: busy"
        ));
        assert_eq!(
            model.failures_left.load(Ordering::Relaxed),
            96,
            "it tried max_retries + 1 times"
        );
    }

    #[tokio::test]
    async fn a_fatal_failure_is_not_retried() {
        let model = Flaky {
            failures_left: AtomicU32::new(99),
            fatal: true,
        };

        let cancel = Cancel::default();
        let result =
            generate_with_retry(&model, request(&cancel), &Tuning::instant(), &Quiet).await;

        assert!(matches!(result, Err(CallError::Fatal(_))));
        assert_eq!(
            model.failures_left.load(Ordering::Relaxed),
            98,
            "it stopped after the first attempt"
        );
    }

    #[tokio::test]
    async fn stopping_the_run_ends_the_retries_at_once() {
        let model = Flaky {
            failures_left: AtomicU32::new(99),
            fatal: false,
        };
        let cancel = Cancel::default();
        cancel.stop();

        let result =
            generate_with_retry(&model, request(&cancel), &Tuning::instant(), &Quiet).await;

        assert!(matches!(result, Err(CallError::Stopped)));
        assert_eq!(
            model.failures_left.load(Ordering::Relaxed),
            99,
            "no attempt was made after the stop"
        );
    }

    #[tokio::test]
    async fn a_call_still_in_flight_is_abandoned_by_the_stop() {
        let cancel = Cancel::default();

        let waiting = until_stopped(&cancel, future::pending::<()>());
        let stopper = async {
            tokio::task::yield_now().await;
            cancel.stop();
        };

        let (result, ()) = tokio::join!(waiting, stopper);

        assert!(
            matches!(result, Err(CallError::Stopped)),
            "a request that never answers has to be dropped, not waited out"
        );
    }

    #[test]
    fn a_rate_limit_or_a_server_fault_is_worth_another_try() {
        for status in [
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::GATEWAY_TIMEOUT,
        ] {
            assert!(
                matches!(classify_status(status, "busy"), CallError::Retryable { .. }),
                "{status} should be retried"
            );
        }
    }

    #[test]
    fn an_overflowing_request_is_blocked_so_the_batch_splits() {
        assert!(matches!(
            classify_status(StatusCode::PAYLOAD_TOO_LARGE, "too big"),
            CallError::Blocked(_)
        ));
        assert!(matches!(
            classify_status(
                StatusCode::BAD_REQUEST,
                "This model's maximum context length is 4096 tokens"
            ),
            CallError::Blocked(_)
        ));
        assert!(
            !matches!(
                classify_status(StatusCode::BAD_REQUEST, "invalid request"),
                CallError::Blocked(_)
            ),
            "a refusal that says nothing about length must not be read as one"
        );
    }
}
