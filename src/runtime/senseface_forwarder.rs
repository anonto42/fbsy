#!/usr/bin/env rust
//! Enhanced SenseFace Forwarder Testing and Monitoring
//!
//! This module provides comprehensive testing capabilities for the SenseFace
//! forwarder with emphasis on logging, observability, and traceability.
//!
//! Key features:
//! - Trace ID generation and propagation
//! - Processing metrics collection
//! - Error categorization and logging
//! - Performance monitoring
//! - Debug trace analysis
//!

use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::time::timeout;

use crate::{
    domain::senseface::SenseFaceAttendanceRecord,
    ports::hrms::ForwardEventResult,
};

/// Forwarder metrics collection for monitoring
#[derive(Debug, Clone)]
pub struct ForwarderMetrics {
    /// Total events received
    pub total_events_received: u64,
    /// Events successfully forwarded
    pub events_forwarded: u64,
    /// Events that failed to forward
    pub events_failed: u64,
    /// Events filtered out (no mapping, invalid)
    pub events_filtered_out: u64,
    /// Device resolution attempts that found mappings
    pub device_resolution_looked_up: u64,
    /// Device resolution attempts that used fallback
    pub device_resolution_fallback_used: u64,
    /// Average processing time in milliseconds
    pub average_processing_time_ms: f64,
    /// Last metrics update timestamp
    pub last_update: std::time::SystemTime,
    /// Processing distribution by device
    pub device_distribution: HashMap<String, DeviceMetrics>,
    /// Processing distribution by error type
    pub error_distribution: HashMap<String, u64>,
}

#[derive(Debug, Clone)]
pub struct DeviceMetrics {
    pub total_events: u64,
    pub processed_events: u64,
    pub failed_events: u64,
    pub device_code: String,
    pub organization_id: u64,
}

/// Forward trace for end-to-end event tracking
#[derive(Debug, Clone)]
pub struct ForwardTrace {
    /// Unique trace identifier
    pub trace_id: String,
    /// Event timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Source device serial number
    pub source_device: String,
    /// Comma-separated employee IDs for batch events
    pub employee_ids: String,
    /// Original event type
    pub original_event_type: String,
    /// Processing step where trace was recorded
    pub processing_step: String,
    /// Processing result
    pub result: String,
    /// Error message if processing failed
    pub error_message: Option<String>,
    /// Processing duration in milliseconds
    pub duration_ms: u64,
    /// Batch trace identifier for related events
    pub batch_trace_id: Option<String>,
}

/// Helper functions for testing and monitoring
impl ForwarderMetrics {
    /// Create new metrics instance
    pub fn new() -> Self {
        Self {
            total_events_received: 0,
            events_forwarded: 0,
            events_failed: 0,
            events_filtered_out: 0,
            device_resolution_looked_up: 0,
            device_resolution_fallback_used: 0,
            average_processing_time_ms: 0.0,
            last_update: std::time::SystemTime::now(),
            device_distribution: HashMap::new(),
            error_distribution: HashMap::new(),
        }
    }

    /// Record a new event received
    pub fn record_event_received(&mut self, device_code: &str) {
        self.total_events_received += 1;
        let device_metrics = self.device_distribution
            .entry(device_code.to_string())
            .or_insert(DeviceMetrics {
                total_events: 0,
                processed_events: 0,
                failed_events: 0,
                device_code: device_code.to_string(),
                organization_id: 0,
            });
        device_metrics.total_events += 1;
        self.last_update = std::time::SystemTime::now();
    }

    /// Record successful event processing
    pub fn record_event_forwarded(&mut self, device_code: &str) {
        self.events_forwarded += 1;
        if let Some(device_metrics) = self.device_distribution.get_mut(device_code) {
            device_metrics.processed_events += 1;
        }
        self.update_average_processing_time(100.0); // Assume 100ms per event
        self.last_update = std::time::SystemTime::now();
    }

    /// Record failed event processing
    pub fn record_event_failed(&mut self, device_code: &str, error_type: &str) {
        self.events_failed += 1;
        if let Some(device_metrics) = self.device_distribution.get_mut(device_code) {
            device_metrics.failed_events += 1;
        }
        *self.error_distribution.entry(error_type.to_string()).or_insert(0) += 1;
        self.update_average_processing_time(200.0); // Assume 200ms for failed events
        self.last_update = std::time::SystemTime::now();
    }

    /// Record filtered out events
    pub fn record_events_filtered_out(&mut self, count: u64) {
        self.events_filtered_out += count;
        self.last_update = std::time::SystemTime::now();
    }

    /// Record device resolution outcomes
    pub fn record_device_resolution(
        &mut self,
        looked_up: bool,
        used_fallback: bool,
        device_code: &str,
        organization_id: u64,
    ) {
        if looked_up {
            self.device_resolution_looked_up += 1;
            if let Some(device_metrics) = self.device_distribution.get_mut(device_code) {
                device_metrics.organization_id = organization_id;
            }
        } else if used_fallback {
            self.device_resolution_fallback_used += 1;
            let device_metrics = self.device_distribution
                .entry(device_code.to_string())
                .or_insert(DeviceMetrics {
                    total_events: 0,
                    processed_events: 0,
                    failed_events: 0,
                    device_code: device_code.to_string(),
                    organization_id,
                });
            device_metrics.total_events = 1;
        }
    }

    /// Update running average processing time
    fn update_average_processing_time(&mut self, new_time_ms: f64) {
        if self.total_events_received == 0 {
            self.average_processing_time_ms = new_time_ms;
        } else {
            let old_count = self.total_events_received - 1;
            self.average_processing_time_ms = 
                (self.average_processing_time_ms * old_count as f64 + new_time_ms) / 
                (old_count + 1) as f64;
        }
    }

    /// Generate metrics report
    pub fn generate_report(&self) -> String {
        let success_rate = if self.total_events_received > 0 {
            (self.events_forwarded as f64 / self.total_events_received as f64) * 100.0
        } else {
            0.0
        };

        let mut report = String::new();
        report.push_str("=== SenseFace Forwarder Metrics Report ===\n");
        report.push_str(&format!("Total Events Received: {}\n", self.total_events_received));
        report.push_str(&format!("Events Successfully Forwarded: {}\n", self.events_forwarded));
        report.push_str(&format!("Events Failed: {}\n", self.events_failed));
        report.push_str(&format!("Events Filtered Out: {}\n", self.events_filtered_out));
        report.push_str(&format!("Success Rate: {:.1}%", success_rate));
        report.push_str("\n\n--- Device Distribution ---\n");
        for (device_code, metrics) in &self.device_distribution {
            let device_success_rate = if metrics.total_events > 0 {
                (metrics.processed_events as f64 / metrics.total_events as f64) * 100.0
            } else {
                0.0
            };
            report.push_str(&format!(
                "  Device {}: {} events ({} processed, {} failed) - {:.1}% success rate",
                device_code,
                metrics.total_events,
                metrics.processed_events + metrics.failed_events,
                metrics.failed_events,
                device_success_rate
            ));
            if metrics.organization_id > 0 {
                report.push_str(&format!(" [org: {}]", metrics.organization_id));
            }
            report.push_str("\n");
        }
        report.push_str("\n--- Error Distribution ---\n");
        for (error_type, count) in &self.error_distribution {
            report.push_str(&format!("  {}: {} occurrences\n", error_type, count));
        }
        report.push_str("\n--- Resolution Statistics ---\n");
        report.push_str(&format!("  Explicit resolutions (looked up): {}\n", self.device_resolution_looked_up));
        report.push_str(&format!("  Fallback resolutions: {}\n", self.device_resolution_fallback_used));
        report.push_str(&format!("  Average Processing Time: {:.1f} ms\n", self.average_processing_time_ms));
        report.push_str(&format!("  Last Update: {:?}\n", self.last_update));

        report
    }
}

/// Trace ID generation utilities
pub struct TraceUtils;

impl TraceUtils {
    /// Generate a unique trace ID for event tracking
    pub fn generate_trace_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    /// Create a batch trace ID
    pub fn create_batch_trace_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    /// Parse trace ID from string
    pub fn parse_trace_id(trace_id: &str) -> Result<uuid::Uuid, uuid::Error> {
        uuid::Uuid::parse_str(trace_id)
    }

    /// Extract device from trace ID if present
    pub fn extract_device_from_trace(trace_id: &str) -> Option<String> {
        if let Ok(parsed) = TraceUtils::parse_trace_id(trace_id) {
            let uuid_str = parsed.to_string();
            // Extract device info from UUID if available (simplified)
            if uuid_str.len() > 8 {
                return Some(format!("device-{}", &uuid_str[..8]));
            }
        }
        None
    }
}

/// Performance benchmarking utilities
pub struct PerformanceBenchmark;

impl PerformanceBenchmark {
    /// Measure execution time of a function
    pub fn measure_execution<F, R>(func: F) -> (R, std::time::Duration)
    where
        F: FnOnce() -> R,
    {
        let start = std::time::Instant::now();
        let result = func();
        let duration = start.elapsed();
        (result, duration)
    }

    /// Benchmark multiple runs and return average
    pub fn benchmark_multiple<F, R>(func: F, runs: usize) -> (R, f64)
    where
        F: Fn() -> R + Copy,
    {
        let mut times = Vec::with_capacity(runs);
        let mut last_result = None;
        
        for _ in 0..runs {
            let (result, duration) = Self::measure_execution(func);
            times.push(duration.as_millis() as f64);
            last_result = Some(result);
        }
        
        let average_time = times.iter().sum::<f64>() / times.len() as f64;
        let std_dev = if times.len() > 1 {
            let variance: f64 = times.iter()
                .map(|t| (t - average_time).powi(精准性)
                .sum::<f64>() / (times.len() - 1) as f64;
            variance.sqrt()
        } else {
            0.0
        };
        
        (last_result.unwrap(), average_time)
    }

    /// Calculate percentile from timing measurements
    pub fn calculate_percentile(times: &[f64], percentile: f64) -> f64 {
        if times.is_empty() {
            return 0.0;
        }
        let mut sorted = times.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let index = (percentile / 100.0 * sorted.len() as f64).ceil() as usize - 1;
        if index >= sorted.len() {
            sorted[sorted.len() - 1]
        } else {
            sorted[index]
        }
    }
}

/// Log analysis utilities
pub struct LogAnalysis;

impl LogAnalysis {
    /// Filter traces by trace ID
    pub fn filter_traces_by_batch_id(traces: &[ForwardTrace], batch_id: &str) -> Vec<ForwardTrace> {
        traces.iter()
            .filter(|trace| trace.batch_trace_id.as_ref() == Some(&batch_id.to_string()))
            .cloned()
            .collect()
    }

    /// Group traces by source device
    pub fn group_traces_by_device(traces: &[ForwardTrace]) -> HashMap<String, Vec<ForwardTrace>> {
        let mut grouped: HashMap<String, Vec<ForwardTrace>> = HashMap::new();
        for trace in traces {
            grouped
                .entry(trace.source_device.clone())
                .or_default()
                .push(trace.clone());
        }
        grouped
    }

    /// Analyze error patterns in traces
    pub fn analyze_error_patterns(traces: &[ForwardTrace]) -> Vec<ErrorPattern> {
        let mut error_patterns: Vec<ErrorPattern> = Vec::new();
        
        let mut error_counts: HashMap<String, usize> = HashMap::new();
        for trace in traces {
            if let Some(error_msg) = &trace.error_message {
                *error_counts.entry(error_msg.clone()).or_insert(0) += 1;
            }
        }
        
        for (error, count) in error_counts.iter() {
            error_patterns.push(ErrorPattern {
                error_message: error.clone(),
                occurrence_count: *count,
                percentage: if !traces.is_empty() {
                    (*count as f64 / traces.len() as f64) * 100.0
                } else {
                    0.0
                },
            });
        }
        
        error_patterns.sort_by(|a, b| b.occurrence_count.cmp(&a.occurrence_count));
        error_patterns
    }

    /// Identify slow processing traces
    pub fn identify_slow_traces(
        traces: &[ForwardTrace],
        threshold_ms: u64,
    ) -> Vec<ForwardTrace> {
        traces.iter()
            .filter(|trace| trace.duration_ms > threshold_ms)
            .cloned()
            .collect()
    }

    /// Generate processing heatmap for timeline analysis
    pub fn generate_timeline_heatmap(
        traces: &[ForwardTrace],
        window_minutes: i64,
    ) -> HashMap<String, usize> {
        let mut heatmap: HashMap<String, usize> = HashMap::new();
        let start_time = chrono::Utc::now() - chrono::Duration::minutes(window_minutes);
        
        for trace in traces {
            if trace.timestamp >= start_time {
                let hour_key = trace.timestamp.format("%Y-%m-%d %H:00").to_string();
                *heatmap.entry(hour_key).or_insert(0) += 1;
            }
        }
        
        heatmap
    }
}

/// Error pattern analysis
#[derive(Debug, Clone)]
pub struct ErrorPattern {
    /// Error message
    pub error_message: String,
    /// Number of occurrences
    pub occurrence_count: usize,
    /// Percentage of total traces
    pub percentage: f64,
}

/// Performance alert system
pub struct PerformanceAlerts;

impl PerformanceAlerts {
    /// Check for high error rate alerts
    pub fn check_high_error_rate(
        traces: &[ForwardTrace],
        threshold_percentage: f64,
    ) -> Result<(), String> {
        let total_traces = traces.len();
        if total_traces == 0 {
            return Ok(());
        }
        
        let error_traces = traces.iter()
            .filter(|t| t.result == "failed")
            .count();
        
        let error_rate = (error_traces as f64 / total_traces as f64) * 100.0;
        
        if error_rate > threshold_percentage {
            return Err(format!(
                "High error rate detected: {:.1}% (threshold: {:.1}%)",
                error_rate, threshold_percentage
            ));
        }
        
        Ok(())
    }

    /// Check for device resolution issues
    pub fn check_device_resolution_issues(
        traces: &[ForwardTrace],
        fallback_rate_threshold: f64,
    ) -> Result<(), String> {
        let mut total_resolutions = 0;
        let mut fallback_resolutions = 0;
        
        for trace in traces {
            if trace.processing_step.contains("device_resolution") {
                total_resolutions += 1;
                if trace.result.contains("fallback") {
                    fallback_resolutions += 1;
                }
            }
        }
        
        if total_resolutions > 0 {
            let fallback_rate = (fallback_resolutions as f64 / total_resolutions as f64) * 100.0;
            
            if fallback_rate > fallback_rate_threshold {
                return Err(format!(
                    "High device resolution fallback rate: {:.1}% (threshold: {:.1}%)",
                    fallback_rate, fallback_rate_threshold
                ));
            }
        }
        
        Ok(())
    }

    /// Check for performance degradation
    pub fn check_performance_degradation(
        recent_traces: &[ForwardTrace],
        older_traces: &[ForwardTrace],
        performance_threshold_ms: u64,
    ) -> Result<(), String> {
        if recent_traces.is_empty() || older_traces.is_empty() {
            return Ok(());
        }
        
        let recent_avg = recent_traces.iter()
            .map(|t| t.duration_ms)
            .sum::<u64>() as f64 /
            recent_traces.len() as f64;
        
        let older_avg = older_traes.iter()
            .map(|t| t.duration_ms)
            .sum::<u64>() as f64 /
            older_traces.len() as f64;
        
        if recent_avg > older_avg * 1.5 && recent_avg > performance_threshold_ms as f64 {
            return Err(format!(
                "Performance degradation detected: recent avg {:.1f}ms vs older avg {:.1f}ms (threshold: {}ms)",
                recent_avg, older_avg, performance_threshold_ms
            ));
        }
        
        Ok(())
    }
}

/// Metrics persistence and retrieval
pub struct MetricsPersistence;

impl MetricsPersistence {
    /// Save metrics to file
    pub fn save_metrics_to_file(metrics: &ForwarderMetrics, path: &str) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(metrics)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(path, json)
    }

    /// Load metrics from file
    pub fn load_metrics_from_file(path: &str) -> std::io::Result<ForwarderMetrics> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        let metrics: ForwarderMetrics = serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        Ok(metrics)
    }

    /// Export traces to file for analysis
    pub fn save_traces_to_file(traces: &[ForwardTrace], path: &str) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(traces)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(path, json)
    }
}

/// Integration with observability systems
pub struct ObservabilityIntegration;

impl ObservabilityIntegration {
    /// Send metrics to OpenTelemetry
    pub fn export_metrics_to_otel(metrics: &ForwarderMetrics) {
        // In a real implementation, this would use OpenTelemetry libraries
        // to export metrics to observability systems like Prometheus,
        // Jaeger, or other telemetry backends.
        
        let _ = metrics; // Silences unused variable warning
                          
        // Example metrics that could be exported:
        // - total_events_received (counter)
        // - average_processing_time_ms (histogram)
        // - events_forwarded (counter)
        // - events_failed (counter)
        // - device_resolution_looked_up (counter)
        // - device_resolution_fallback_used (counter)
    }

    /// Send traces to distributed tracing system
    pub fn export_traces_to_traceservice(traces: &[ForwardTrace]) {
        // In a real implementation, this would use OpenTelemetry libraries
        // to export traces to distributed tracing systems like Jaeger or Zipkin.
        
        let _ = traces; // Silences unused variable warning
    }

    /// Send logs to logging system
    pub fn export_logs_to_logging_system(traces: &[ForwardTrace], metrics: &ForwarderMetrics) {
        // In a real implementation, this would use logging libraries
        // to send structured logs to systems like ELK stack, Splunk, or similar.
        
        let _ = (traces, metrics); // Silences unused variable warning
    }
}

/// Testing utilities for senseface forwarder
pub struct SensefaceForwarderTesting;

impl SensefaceForwarderTesting {
    /// Generate synthetic test data
    pub fn generate_test_traces(count: usize) -> Vec<ForwardTrace> {
        let mut traces = Vec::with_capacity(count);
        let device_codes = vec!["SF-001", "SF-002", "SF-003", "SF-004", "SF-005"];
        
        for i in 0..count {
            let trace_id = TraceUtils::generate_trace_id();
            let batch_id = TraceUtils::create_batch_trace_id();
            
            let device_code = device_codes[i % device_codes.len()].to_string();
            let employee_id = format!("EMP-{}", (i % 100) + 1);
            let event_type = if i % 3 == 0 {
                "check_in".to_string()
            } else if i % 3 == 1 {
                "check_out".to_string()
            } else {
                "unknown".to_string()
            };
            
            let duration_ms = if i % 10 == 0 { 5000 } else { 100 }; // Some slow operations
            let is_error = i % 15 == 0; // 6.67% error rate
            
            let trace = ForwardTrace {
                trace_id,
                timestamp: chrono::Utc::now() - chrono::Duration::minutes((count - i) as i64),
                source_device: device_code.clone(),
                employee_ids: employee_id,
                original_event_type: event_type.clone(),
                processing_step: if event_type == "check_in" {
                    "device_resolution_events_created"
                } else if event_type == "check_out" {
                    "hrms_forward"
                } else {
                    "hrms_error"
                }.to_string(),
                result: if is_error {
                    "failed".to_string()
                } else {
                    "success".to_string()
                },
                error_message: if is_error {
                    Some(format!("Test error for event {}", i))
                } else {
                    None
                },
                duration_ms,
                batch_trace_id: Some(batch_id.clone()),
            };
            
            traces.push(trace);
        }
        
        traces
    }

    /// Run unit tests for trace analysis
    pub fn run_trace_analysis_tests() {
        let traces = SensefaceForwarderTesting::generate_test_traces(100);
        
        // Test trace filtering
        let device_traces = LogAnalysis::group_traces_by_device(&traces);
        assert!(!device_traces.is_empty(), "Should have traces grouped by device");
        
        // Test error pattern analysis
        let error_patterns = LogAnalysis::analyze_error_patterns(&traces);
        println!("Error patterns found: {}", error_patterns.len());
        
        // Test slow trace identification
        let slow_traces = LogAnalysis::identify_slow_traces(&traces, 1000);
        println!("Slow traces identified: {}", slow_traces.len());
        
        // Test metrics
        let mut metrics = ForwarderMetrics::new();
        for trace in &traces {
            metrics.record_event_received(&trace.source_device);
            if trace.result == "success" {
                metrics.record_event_forwarded(&trace.source_device);
            } else {
                metrics.record_event_failed(&trace.source_device, 
                    trace.error_message.as_deref().unwrap_or("unknown"));
            }
        }
        
        let report = metrics.generate_report();
        println!("Generated test metrics report:\n{}", report);
    }

    /// Run performance benchmarking tests
    pub fn run_performance_benchmarks() {
        let traces = SensefaceForwarderTesting::generate_test_traces(1000);
        
        println!("Running performance benchmarks...")
        
        // Benchmark trace filtering
        let start = std::time::Instant::now();
        for _ in 0..10 {
            let _ = LogAnalysis::group_traces_by_device(&traces);
        }
        let duration_ms = start.elapsed().as_millis() as f64;
        println!("Trace grouping benchmark: {:.2}ms", duration_ms);
        
        // Benchmark error pattern analysis
        let start = std::time::Instant::now();
        for _ in 0..10 {
            let _ = LogAnalysis::analyze_error_patterns(&traces);
        }
        let duration_ms = start.elapsed().as_millis() as f64;
        println!("Error pattern analysis benchmark: {:.2}ms", duration_ms);
        
        // Benchmark slow trace identification
        let start = std::time::Instant::now();
        for _ in 0..10 {
            let _ = LogAnalysis::identify_slow_traces(&traces, 1000);
        }
        let duration_ms = start.elapsed().as_millis() as f64;
        println!("Slow trace identification benchmark: {:.2}ms", duration_ms);
    }
}