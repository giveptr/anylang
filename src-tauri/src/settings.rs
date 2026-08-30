use crate::llm::Provider;
use crate::store;
use crate::tuning::{self, Tuning};
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::BTreeMap;
use std::path::PathBuf;
use tokio::fs;

const FILE: &str = "settings.json";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct Temperature(String);

impl Temperature {
    pub fn heated(&self) -> Result<Option<f32>> {
        let said = self.0.trim();
        if said.is_empty() {
            return Ok(None);
        }

        said.parse::<f32>()
            .ok()
            .filter(|heat| heat.is_finite())
            .map(Some)
            .ok_or_else(|| anyhow!("{said} is not a temperature."))
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
    pub temperature: Temperature,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub credentials: String,
    pub model: String,
    pub temperature: Temperature,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Endpoint {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub temperature: Temperature,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub using: Provider,
    pub preset: String,
    pub gemini: Sampled,
    pub vertex: Account,
    pub claude: Keyed,
    pub endpoints: BTreeMap<String, Endpoint>,
    pub lines_per_request: u32,
    pub parallel_requests: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            using: Provider::default(),
            preset: String::new(),
            gemini: Sampled::default(),
            vertex: Account::default(),
            claude: Keyed::default(),
            endpoints: BTreeMap::default(),
            lines_per_request: tuning::LINES_PER_REQUEST,
            parallel_requests: tuning::PARALLEL_REQUESTS,
        }
    }
}

impl Settings {
    pub fn endpoint(&self) -> Endpoint {
        self.endpoints
            .get(&self.preset)
            .cloned()
            .unwrap_or_default()
    }

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

    Ok(serde_json::from_str(&raw).unwrap_or_default())
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
    fn a_field_left_empty_lets_the_model_keep_its_own_default() {
        assert_eq!(Temperature("0.7".to_string()).heated().unwrap(), Some(0.7));
        assert_eq!(
            Temperature::default().heated().unwrap(),
            None,
            "an endpoint that turns the whole request down for carrying a temperature has to \
             be left alone, and no number says that"
        );
        assert!(
            Temperature("inf".to_string()).heated().is_err(),
            "a number that is not one reaches the endpoint as a null temperature, which is not \
             what the reader typed and not what the model would have picked"
        );
        assert!(
            Temperature("hot".to_string()).heated().is_err(),
            "a word where a number belongs is a mistake worth naming, not one to drop on the \
             floor and translate a whole game without"
        );
    }
}
