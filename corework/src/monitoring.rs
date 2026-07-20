//! Lightweight telemetry and in-memory metrics primitives.
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

/// Telemetry sink used by framework contexts and systems.
#[async_trait]
pub trait Telemetry: Send + Sync {
    fn counter(&self, name: &str, value: u64, tags: &[(&str, &str)]);

    fn histogram(&self, name: &str, value: f64, tags: &[(&str, &str)]);

    fn gauge(&self, name: &str, value: f64, tags: &[(&str, &str)]);

    fn start_span(&self, name: &str) -> Span;
}

pub struct Span {
    name: String,
    start_time: Instant,
    tags: HashMap<String, String>,
}

impl Span {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            start_time: Instant::now(),
            tags: HashMap::new(),
        }
    }

    pub fn set_tag(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.tags.insert(key.into(), value.into());
    }

    pub fn duration_ms(&self) -> u64 {
        self.start_time.elapsed().as_millis() as u64
    }

    pub fn end(self) {
        tracing::debug!(
            span = %self.name,
            duration_ms = self.duration_ms(),
            "span completed"
        );
    }
}

pub struct Metrics {
    counters: Arc<parking_lot::RwLock<HashMap<String, u64>>>,
    gauges: Arc<parking_lot::RwLock<HashMap<String, f64>>>,
    histograms: Arc<parking_lot::RwLock<HashMap<String, Vec<f64>>>>,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            counters: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            gauges: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            histograms: Arc::new(parking_lot::RwLock::new(HashMap::new())),
        }
    }

    pub fn increment(&self, name: &str, value: u64) {
        let mut counters = self.counters.write();
        *counters.entry(name.to_string()).or_insert(0) += value;
    }

    pub fn set_gauge(&self, name: &str, value: f64) {
        let mut gauges = self.gauges.write();
        gauges.insert(name.to_string(), value);
    }

    pub fn record_histogram(&self, name: &str, value: f64) {
        let mut histograms = self.histograms.write();
        histograms.entry(name.to_string()).or_default().push(value);
    }

    pub fn get_counter(&self, name: &str) -> Option<u64> {
        self.counters.read().get(name).copied()
    }

    pub fn get_gauge(&self, name: &str) -> Option<f64> {
        self.gauges.read().get(name).copied()
    }

    pub fn get_histogram_stats(&self, name: &str) -> Option<HistogramStats> {
        let histograms = self.histograms.read();
        histograms.get(name).map(|values| {
            let mut sorted = values.clone();
            sorted.sort_by(|a, b| a.total_cmp(b));

            let sum: f64 = sorted.iter().sum();
            let count = sorted.len();
            let mean = sum / count as f64;

            let p50_idx = (count as f64 * 0.50) as usize;
            let p95_idx = (count as f64 * 0.95) as usize;
            let p99_idx = (count as f64 * 0.99) as usize;

            HistogramStats {
                count,
                sum,
                mean,
                min: sorted[0],
                max: sorted[count - 1],
                p50: sorted[p50_idx],
                p95: sorted[p95_idx],
                p99: sorted[p99_idx],
            }
        })
    }

    pub fn reset(&self) {
        self.counters.write().clear();
        self.gauges.write().clear();
        self.histograms.write().clear();
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct HistogramStats {
    pub count: usize,
    pub sum: f64,
    pub mean: f64,
    pub min: f64,
    pub max: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
}

pub struct NoopTelemetry;

#[async_trait]
impl Telemetry for NoopTelemetry {
    fn counter(&self, _name: &str, _value: u64, _tags: &[(&str, &str)]) {}

    fn histogram(&self, _name: &str, _value: f64, _tags: &[(&str, &str)]) {}

    fn gauge(&self, _name: &str, _value: f64, _tags: &[(&str, &str)]) {}

    fn start_span(&self, name: &str) -> Span {
        Span::new(name)
    }
}

/// Telemetry implementation that records metrics and logs observations.
pub struct LoggingTelemetry {
    metrics: Metrics,
}

impl LoggingTelemetry {
    pub fn new() -> Self {
        Self {
            metrics: Metrics::new(),
        }
    }

    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }
}

impl Default for LoggingTelemetry {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Telemetry for LoggingTelemetry {
    fn counter(&self, name: &str, value: u64, tags: &[(&str, &str)]) {
        self.metrics.increment(name, value);
        tracing::debug!("Counter: {} = {} {:?}", name, value, tags);
    }

    fn histogram(&self, name: &str, value: f64, tags: &[(&str, &str)]) {
        self.metrics.record_histogram(name, value);
        tracing::debug!("Histogram: {} = {} {:?}", name, value, tags);
    }

    fn gauge(&self, name: &str, value: f64, tags: &[(&str, &str)]) {
        self.metrics.set_gauge(name, value);
        tracing::debug!("Gauge: {} = {} {:?}", name, value, tags);
    }

    fn start_span(&self, name: &str) -> Span {
        tracing::debug!(span = %name, "span started");
        Span::new(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics() {
        let metrics = Metrics::new();

        metrics.increment("requests", 1);
        metrics.increment("requests", 2);
        assert_eq!(metrics.get_counter("requests"), Some(3));

        metrics.set_gauge("cpu_usage", 75.5);
        assert_eq!(metrics.get_gauge("cpu_usage"), Some(75.5));

        metrics.record_histogram("latency", 100.0);
        metrics.record_histogram("latency", 200.0);
        metrics.record_histogram("latency", 150.0);

        let stats = metrics.get_histogram_stats("latency").unwrap();
        assert_eq!(stats.count, 3);
        assert_eq!(stats.min, 100.0);
        assert_eq!(stats.max, 200.0);
    }

    #[test]
    fn test_span() {
        let mut span = Span::new("test_operation");
        span.set_tag("user_id", "123");

        std::thread::sleep(std::time::Duration::from_millis(10));

        let duration = span.duration_ms();
        assert!(duration >= 10);

        span.end();
    }
}
