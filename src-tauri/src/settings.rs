use crate::llm::Provider;
use crate::store;
use crate::tuning::{self, Tuning};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::PathBuf;
use tokio::fs;

const FILE: &str = "settings.json";

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct Temperature(f32);

impl Default for Temperature {
    fn default() -> Self {
        Self(tuning::TEMPERATURE)
    }
}

impl Temperature {
    pub fn heated(self) -> f32 {
        if self.0.is_finite() {
            self.0.clamp(tuning::COOLEST, tuning::HOTTEST)
        } else {
            tuning::TEMPERATURE
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Keyed {
    pub api_key: String,
    pub model: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Sampled {
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub temperature: Temperature,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub credentials: String,
    pub model: String,
    #[serde(default)]
    pub temperature: Temperature,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Endpoint {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub temperature: Temperature,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub using: Provider,
    pub gemini: Sampled,
    pub vertex: Account,
    pub claude: Keyed,
    pub compatible: Endpoint,
    pub lines_per_request: u32,
    pub parallel_requests: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            using: Provider::default(),
            gemini: Sampled::default(),
            vertex: Account::default(),
            claude: Keyed::default(),
            compatible: Endpoint::default(),
            lines_per_request: tuning::LINES_PER_REQUEST,
            parallel_requests: tuning::PARALLEL_REQUESTS,
        }
    }
}

impl Settings {
    pub fn tuning(&self) -> Tuning {
        Tuning {
            lines_per_request: (self.lines_per_request as usize).max(1),
            parallel_requests: (self.parallel_requests as usize).max(1),
            ..Tuning::default()
        }
    }
}

async fn path() -> Result<PathBuf> {
    let dir = store::app_dir()?;

    fs::create_dir_all(&dir)
        .await
        .with_context(|| format!("creating {}", dir.display()))?;

    Ok(dir.join(FILE))
}

pub async fn load() -> Result<Settings> {
    let file = path().await?;
    let Some(raw) = store::read_if_there(&file).await? else {
        return Ok(Settings::default());
    };

    serde_json::from_str(&raw)
        .with_context(|| format!("{} is not readable. Fix or delete it", file.display()))
}

pub async fn save(settings: &Settings) -> Result<()> {
    let file = path().await?;

    store::write_atomically(&file, serde_json::to_vec_pretty(settings)?).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_knobs_are_clamped_to_something_workable() {
        let settings = Settings {
            lines_per_request: 0,
            parallel_requests: 0,
            ..Settings::default()
        };
        let tuning = settings.tuning();

        assert_eq!(tuning.lines_per_request, 1);
        assert_eq!(tuning.parallel_requests, 1);
    }

    #[test]
    fn a_temperature_no_model_would_accept_is_pulled_back_into_range() {
        assert_eq!(Temperature(0.7).heated(), 0.7);
        assert_eq!(Temperature(-1.0).heated(), tuning::COOLEST);
        assert_eq!(Temperature(9.0).heated(), tuning::HOTTEST);
        assert_eq!(
            Temperature(f32::NAN).heated(),
            tuning::TEMPERATURE,
            "a number that is not one falls back to the default rather than to a bound"
        );
    }
}
