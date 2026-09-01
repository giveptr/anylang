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
use reqwest::{Client, RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use specta::Type;
use std::fmt;
use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
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

pub const ANSWERS: &str = "items";

#[derive(Debug, Clone, Copy)]
pub enum Spelling {
    Plain,
    Closed,
    Shouted,
}

fn kind(name: &str, spelling: Spelling) -> Value {
    match spelling {
        Spelling::Shouted => Value::String(name.to_uppercase()),
        Spelling::Plain | Spelling::Closed => Value::String(name.to_string()),
    }
}

fn shut(mut object: Value, closed: bool) -> Value {
    if closed {
        object["additionalProperties"] = Value::Bool(false);
    }

    object
}

pub fn answer_schema(spelling: Spelling) -> Value {
    let closed = matches!(spelling, Spelling::Closed);

    let line = shut(
        json!({
            "type": kind("object", spelling),
            "properties": {
                "id": { "type": kind("integer", spelling) },
                "translation": { "type": kind("string", spelling) },
            },
            "required": ["id", "translation"],
        }),
        closed,
    );

    shut(
        json!({
            "type": kind("object", spelling),
            "properties": {
                ANSWERS: {
                    "type": kind("array", spelling),
                    "items": line,
                },
            },
            "required": [ANSWERS],
        }),
        closed,
    )
}

#[derive(Debug)]
pub struct Shaping(AtomicBool);

impl Default for Shaping {
    fn default() -> Self {
        Self(AtomicBool::new(true))
    }
}

impl Shaping {
    pub fn wanted(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    pub fn give_up(&self) {
        self.0.store(false, Ordering::Relaxed);
    }
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

    fn reachable<'a>(&'a self, cancel: &'a Cancel) -> BoxFuture<'a, Result<(), CallError>>;
}

pub trait Speaks: Send + Sync {
    fn call(
        &self,
        request: Request<'_>,
    ) -> impl Future<Output = Result<Generation, CallError>> + Send;

    fn reach(&self, cancel: &Cancel) -> impl Future<Output = Result<(), CallError>> + Send;
}

impl<T: Speaks> Model for T {
    fn generate<'a>(
        &'a self,
        request: Request<'a>,
    ) -> BoxFuture<'a, Result<Generation, CallError>> {
        self.call(request).boxed()
    }

    fn reachable<'a>(&'a self, cancel: &'a Cancel) -> BoxFuture<'a, Result<(), CallError>> {
        self.reach(cancel).boxed()
    }
}

async fn knocked(outgoing: RequestBuilder, cancel: &Cancel) -> Result<(), CallError> {
    sent(outgoing, cancel, None).await.map(|_| ())
}

async fn sent(
    outgoing: RequestBuilder,
    cancel: &Cancel,
    shaping: Option<&Shaping>,
) -> Result<Response, CallError> {
    let response = until_stopped(cancel, outgoing.send())
        .await?
        .map_err(transport_error)?;

    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }

    let body = until_stopped(cancel, response.text())
        .await?
        .unwrap_or_default();

    if let Some(shaping) = shaping
        && malformed(status, &body)
    {
        shaping.give_up();
    }

    Err(classify_status(status, &body))
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
        .read_timeout(tuning.silence)
        .build()
        .context("building the HTTP client")
}

fn needed(value: &str, what: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{what} is empty.");
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
                it.temperature.heated()?,
            )))
        }
        Provider::Vertex => {
            let it = &settings.vertex;
            Ok(Box::new(
                Gemini::vertex(
                    client,
                    Path::new(&needed(&it.credentials, "The Vertex service account file")?),
                    needed(&it.model, "The model name")?,
                    it.temperature.heated()?,
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
            let it = settings.endpoint();
            Ok(Box::new(OpenAiCompatible::new(
                client,
                needed(&it.base_url, "The endpoint")?,
                needed(&it.model, "The model name")?,
                needed(&it.api_key, "The API key")?,
                it.temperature.heated()?,
            )))
        }
    }
}

async fn streamed<E: DeserializeOwned>(
    outgoing: RequestBuilder,
    cancel: &Cancel,
    shaping: &Shaping,
    mut heard: impl FnMut(E),
) -> Result<(), CallError> {
    let mut response = sent(outgoing, cancel, Some(shaping)).await?;
    let mut sse = Sse::default();

    while let Some(bytes) = until_stopped(cancel, response.chunk())
        .await?
        .map_err(|error| CallError::Retryable(format!("the stream broke off: {error}")))?
    {
        delivered(sse.fed(&bytes)?, &mut heard)?;
    }

    delivered(sse.drained()?, &mut heard)
}

fn delivered<E: DeserializeOwned>(
    data: Vec<String>,
    heard: &mut impl FnMut(E),
) -> Result<(), CallError> {
    for one in data {
        if let Some(event) = parsed(&one)? {
            heard(event);
        }
    }

    Ok(())
}

const DONE: &str = "[DONE]";

fn parsed<E: DeserializeOwned>(data: &str) -> Result<Option<E>, CallError> {
    if data == DONE {
        return Ok(None);
    }

    let value: Value = serde_json::from_str(data)
        .map_err(|error| CallError::Retryable(format!("an event was not valid JSON: {error}")))?;
    if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
        return Err(CallError::Retryable(format!(
            "the stream carried an error: {error}"
        )));
    }

    serde_json::from_value(value).map(Some).map_err(|error| {
        CallError::Retryable(format!("an event was not the shape expected: {error}"))
    })
}

#[derive(Default)]
struct Sse {
    pending: Vec<u8>,
    data: Vec<String>,
}

impl Sse {
    fn fed(&mut self, bytes: &[u8]) -> Result<Vec<String>, CallError> {
        self.pending.extend_from_slice(bytes);

        let mut out = Vec::new();
        while let Some(at) = self.pending.iter().position(|&byte| byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=at).collect();
            out.extend(self.line(&line)?);
        }

        Ok(out)
    }

    fn drained(&mut self) -> Result<Vec<String>, CallError> {
        let mut out = Vec::new();

        let rest = std::mem::take(&mut self.pending);
        if !rest.is_empty() {
            out.extend(self.line(&rest)?);
        }
        out.extend(self.flushed());

        Ok(out)
    }

    fn line(&mut self, raw: &[u8]) -> Result<Option<String>, CallError> {
        let line = std::str::from_utf8(raw)
            .map_err(|_| CallError::Retryable("the stream was not UTF-8".to_string()))?
            .trim_end_matches(['\r', '\n']);

        if line.is_empty() {
            return Ok(self.flushed());
        }

        if let Some(rest) = line.strip_prefix("data:") {
            self.data
                .push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        }

        Ok(None)
    }

    fn flushed(&mut self) -> Option<String> {
        if self.data.is_empty() {
            return None;
        }

        Some(std::mem::take(&mut self.data).join("\n"))
    }
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

fn malformed(status: StatusCode, body: &str) -> bool {
    matches!(
        status,
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY
    ) && !overflows(status, body)
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
            "HTTP {status} - model or endpoint not found, check the model name and the base \
             URL\n{snippet}"
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
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

        async fn reach(&self, _cancel: &Cancel) -> Result<(), CallError> {
            Ok(())
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

        async fn reach(&self, _cancel: &Cancel) -> Result<(), CallError> {
            Ok(())
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
    fn events_are_cut_at_blank_lines_however_the_bytes_arrive() {
        let mut sse = Sse::default();
        let mut got = Vec::new();
        for piece in [
            "data: {\"a\"",
            ":1}\r\n\r\ndata: one\ndata: two\n\n: keep-alive\n\nevent: x\ndata: {\"b\":2}\n",
            "\n",
        ] {
            got.extend(sse.fed(piece.as_bytes()).unwrap());
        }
        got.extend(sse.drained().unwrap());

        assert_eq!(
            got,
            ["{\"a\":1}", "one\ntwo", "{\"b\":2}"],
            "the network hands over bytes at its own boundaries, so one event may arrive in \
             halves and two may arrive at once; only the blank line says where an event ends"
        );
    }

    #[test]
    fn a_last_event_with_no_blank_line_after_it_is_still_delivered() {
        let mut sse = Sse::default();
        assert!(sse.fed(b"data: tail").unwrap().is_empty());
        assert_eq!(
            sse.drained().unwrap(),
            ["tail"],
            "a server that closes right after its last event never sends the blank line"
        );
    }

    #[test]
    fn what_the_stream_says_is_read_and_what_it_signals_is_not() {
        assert!(
            parsed::<Value>(DONE).unwrap().is_none(),
            "the end marker is not an event"
        );
        assert_eq!(parsed::<Value>("{\"n\":1}").unwrap(), Some(json!({"n": 1})));
        assert_eq!(
            parsed::<Value>("{\"n\":1,\"error\":null}").unwrap(),
            Some(json!({"n": 1, "error": null})),
            "some servers spell out an error slot on every chunk and leave it empty"
        );
        assert!(
            matches!(
                parsed::<Value>("{\"error\":{\"message\":\"overloaded\"}}"),
                Err(CallError::Retryable(message)) if message.contains("overloaded")
            ),
            "an error sent after the 200 is the only word the reader gets about why the answer \
             stopped, and reading it as an empty answer would hide it"
        );
    }

    async fn serving(script: Vec<(u64, &'static str)>, held: bool) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a free port");
        let port = listener.local_addr().expect("a bound address").port();

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("one client");
            let mut seen = Vec::new();
            let mut buffer = [0u8; 1024];
            while !seen.windows(4).any(|end| end == b"\r\n\r\n") {
                let read = socket.read(&mut buffer).await.expect("the request");
                if read == 0 {
                    return;
                }
                seen.extend_from_slice(&buffer[..read]);
            }

            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
                )
                .await
                .expect("headers");
            for (pause, piece) in script {
                time::sleep(Duration::from_millis(pause)).await;
                socket.write_all(piece.as_bytes()).await.expect("a piece");
            }
            if held {
                time::sleep(Duration::from_secs(5)).await;
            }
        });

        format!("http://127.0.0.1:{port}/")
    }

    async fn heard_from(url: &str, silence: u64) -> Result<Vec<Value>, CallError> {
        let client = Client::builder()
            .read_timeout(Duration::from_millis(silence))
            .build()
            .expect("a client");

        let mut got = Vec::new();
        streamed::<Value>(
            client.post(url),
            &Cancel::default(),
            &Shaping::default(),
            |event| got.push(event),
        )
        .await?;

        Ok(got)
    }

    #[tokio::test]
    async fn an_answer_that_keeps_arriving_is_never_cut_however_long_it_takes() {
        let url = serving(
            vec![
                (0, "data: {\"n\":1}\n\n"),
                (150, "data: {\"n\":2}\n\n"),
                (150, "data: {\"n\":3}\n\n"),
                (150, "data: [DONE]\n\n"),
            ],
            false,
        )
        .await;

        let got = heard_from(&url, 300)
            .await
            .unwrap_or_else(|error| panic!("{error}"));

        assert_eq!(
            got,
            [json!({"n": 1}), json!({"n": 2}), json!({"n": 3})],
            "450ms of answer under a 300ms silence limit: what is bounded is the quiet between \
             pieces, never the whole, so a long answer from a slow model is not the thing that \
             gets cut"
        );
    }

    #[tokio::test]
    async fn a_stream_that_goes_quiet_is_given_up_after_the_silence_allowed() {
        let url = serving(vec![(0, "data: {\"n\":1}\n\n")], true).await;

        let began = time::Instant::now();
        let result = heard_from(&url, 300).await;

        assert!(
            matches!(result, Err(CallError::Retryable(_))),
            "a proxy that took the request and went silent is told apart from a model still \
             writing by the one thing that differs: nothing arrives"
        );
        assert!(
            began.elapsed() < Duration::from_secs(3),
            "and it is given up as soon as the silence runs out, not when the server lets go"
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

    #[test]
    fn a_request_the_server_calls_malformed_is_the_one_that_stops_the_shaping() {
        assert!(malformed(
            StatusCode::BAD_REQUEST,
            "unknown field response_format"
        ));
        assert!(malformed(
            StatusCode::UNPROCESSABLE_ENTITY,
            "body -> response_format: extra fields not permitted"
        ));
        assert!(
            !malformed(
                StatusCode::BAD_REQUEST,
                "This model's maximum context length is 4096 tokens"
            ),
            "a batch too long for the model says nothing about the schema, and giving the \
             schema up over it would leave the run guessing for no reason"
        );
        for status in [
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::UNAUTHORIZED,
        ] {
            assert!(
                !malformed(status, "busy"),
                "a request worth sending again is not a request the server could not read"
            );
        }
    }
}
