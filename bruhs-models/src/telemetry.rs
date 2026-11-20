//! Owner-configured OTLP/HTTP endpoints for the local task host.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Signal {
    Traces,
    Metrics,
    Logs,
}
impl Signal {
    pub const ALL: [Self; 3] = [Self::Traces, Self::Metrics, Self::Logs];
    pub fn index(self) -> usize {
        match self {
            Self::Traces => 0,
            Self::Metrics => 1,
            Self::Logs => 2,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Traces => "Traces",
            Self::Metrics => "Metrics",
            Self::Logs => "Logs",
        }
    }
    pub fn path(self) -> &'static str {
        match self {
            Self::Traces => "v1/traces",
            Self::Metrics => "v1/metrics",
            Self::Logs => "v1/logs",
        }
    }
    pub fn variable(self) -> &'static str {
        match self {
            Self::Traces => "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
            Self::Metrics => "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
            Self::Logs => "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
        }
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Endpoint {
    pub url: String,
    pub environment: Option<String>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ExportSettings {
    pub signals: [Endpoint; 3],
}
#[derive(Serialize, Deserialize)]
pub struct SetEndpoint {
    pub signal: Signal,
    pub url: String,
}
