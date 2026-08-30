use std::time::Duration;

pub const LINES_PER_REQUEST: u32 = 50;
pub const PARALLEL_REQUESTS: u32 = 20;

#[derive(Debug, Clone)]
pub struct Tuning {
    pub lines_per_request: usize,
    pub parallel_requests: usize,
    pub request_timeout: Duration,
    pub max_retries: u32,
    pub retry_delay: Duration,
    pub repair_rounds: u32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            lines_per_request: LINES_PER_REQUEST as usize,
            parallel_requests: PARALLEL_REQUESTS as usize,
            request_timeout: Duration::from_secs(120),
            max_retries: 5,
            retry_delay: Duration::from_secs(5),
            repair_rounds: 2,
        }
    }
}

#[cfg(test)]
impl Tuning {
    pub fn instant() -> Self {
        Self {
            retry_delay: Duration::from_millis(1),
            ..Self::default()
        }
    }
}
