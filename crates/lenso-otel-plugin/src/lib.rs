//! Optional OpenTelemetry propagation and export Module for Lenso vNext.

mod export;
mod otlp_http;
mod plugin;
mod signal;
mod trace;

pub use export::{
    ExportError, MAX_OTEL_ATTRIBUTE_KEY_BYTES, MAX_OTEL_ATTRIBUTE_VALUE_BYTES, MAX_OTEL_ATTRIBUTES,
    MAX_OTEL_LOG_BODY_BYTES, MAX_OTEL_METRIC_UNIT_BYTES, MAX_OTEL_SIGNAL_ENCODED_BYTES,
    MAX_OTEL_SIGNAL_NAME_BYTES, NoopExporter, OtelExportStats, OtelExporter, TelemetryAdmission,
    TelemetryError, TelemetryHandle,
};
pub use otlp_http::{OtlpHttpExporter, OtlpHttpExporterConfigError};
pub use plugin::{
    DEFAULT_DIAGNOSTIC_QUEUE_CAPACITY, DEFAULT_TELEMETRY_QUEUE_CAPACITY, OTEL_PLUGIN_PACKAGE_ID,
    OTEL_TELEMETRY_CAPABILITY_ID, OTEL_TELEMETRY_DESCRIPTOR_VERSION, OTEL_TELEMETRY_OPERATION,
    OtelPluginConfig, OtelPluginFactory, OtelTelemetry, OtelTelemetryClient,
    TelemetryInvocationError, TelemetryResponse,
};
pub use signal::{OtelLog, OtelMetric, OtelSeverity, OtelSignal, OtelSpan, diagnostic_to_signal};
pub use trace::{
    DEFAULT_TRACE_CONTEXT_ISSUER, TRACE_CONTEXT_EXTENSION_KEY, TraceContext,
    TraceContextConfigError, TraceContextError, TraceContextParseError, TraceContextPropagator,
};
