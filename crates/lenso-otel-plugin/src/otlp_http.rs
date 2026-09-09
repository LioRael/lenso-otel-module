use std::{fmt, time::SystemTime};

use futures::future::LocalBoxFuture;
use opentelemetry_proto::tonic::{
    collector::{logs::v1::ExportLogsServiceRequest, trace::v1::ExportTraceServiceRequest},
    common::v1::{AnyValue, InstrumentationScope, KeyValue, any_value},
    logs::v1::{LogRecord, ResourceLogs, ScopeLogs},
    resource::v1::Resource,
    trace::v1::{ResourceSpans, ScopeSpans, Span, Status, span},
};
use prost::Message as _;
use reqwest::{StatusCode, header};

use crate::{ExportError, OtelExporter, OtelLog, OtelSeverity, OtelSignal, OtelSpan};

const CONTENT_TYPE_PROTOBUF: &str = "application/x-protobuf";
const INSTRUMENTATION_SCOPE_NAME: &str = "lenso-otel-plugin";

/// Invalid Host-private OTLP/HTTP exporter configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OtlpHttpExporterConfigError {
    /// The endpoint is not an absolute HTTP(S) URL without credentials, query, or fragment.
    InvalidEndpoint,
    /// The bearer token is empty or cannot be represented as an HTTP authorization header.
    InvalidBearerToken,
    /// The resource service name is empty.
    InvalidServiceName,
    /// The HTTP client could not be constructed.
    Client,
}

impl fmt::Display for OtlpHttpExporterConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidEndpoint => "invalid OTLP/HTTP endpoint",
            Self::InvalidBearerToken => "invalid OTLP/HTTP bearer token",
            Self::InvalidServiceName => "invalid OTLP service name",
            Self::Client => "OTLP/HTTP client could not be constructed",
        })
    }
}

impl std::error::Error for OtlpHttpExporterConfigError {}

/// A concrete OTLP/HTTP Protobuf exporter configured by the native Host.
///
/// Endpoint credentials and service identity are private adapter inputs. They
/// are not Plugin configuration and never enter the Resolved App Plan.
#[derive(Clone)]
pub struct OtlpHttpExporter {
    client: reqwest::Client,
    traces_endpoint: reqwest::Url,
    logs_endpoint: reqwest::Url,
    authorization: header::HeaderValue,
    service_name: String,
}

impl OtlpHttpExporter {
    /// Creates an exporter for one OTLP/HTTP origin or base path.
    pub fn new(
        endpoint: &str,
        bearer_token: &str,
        service_name: &str,
    ) -> Result<Self, OtlpHttpExporterConfigError> {
        let mut base = reqwest::Url::parse(endpoint)
            .map_err(|_| OtlpHttpExporterConfigError::InvalidEndpoint)?;
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(OtlpHttpExporterConfigError::InvalidEndpoint);
        }
        if !base.path().ends_with('/') {
            let path = format!("{}/", base.path());
            base.set_path(&path);
        }
        let traces_endpoint = base
            .join("v1/traces")
            .map_err(|_| OtlpHttpExporterConfigError::InvalidEndpoint)?;
        let logs_endpoint = base
            .join("v1/logs")
            .map_err(|_| OtlpHttpExporterConfigError::InvalidEndpoint)?;
        if bearer_token.is_empty() {
            return Err(OtlpHttpExporterConfigError::InvalidBearerToken);
        }
        let mut authorization = header::HeaderValue::from_str(&format!("Bearer {bearer_token}"))
            .map_err(|_| OtlpHttpExporterConfigError::InvalidBearerToken)?;
        authorization.set_sensitive(true);
        if service_name.trim().is_empty() {
            return Err(OtlpHttpExporterConfigError::InvalidServiceName);
        }
        let client = reqwest::Client::builder()
            .build()
            .map_err(|_| OtlpHttpExporterConfigError::Client)?;
        Ok(Self {
            client,
            traces_endpoint,
            logs_endpoint,
            authorization,
            service_name: service_name.to_owned(),
        })
    }

    async fn post(&self, endpoint: reqwest::Url, body: Vec<u8>) -> Result<(), ExportError> {
        let result = self
            .client
            .post(endpoint)
            .header(header::CONTENT_TYPE, CONTENT_TYPE_PROTOBUF)
            .header(header::AUTHORIZATION, self.authorization.clone())
            .body(body)
            .send()
            .await
            .map_err(|_| ExportError::Unavailable)?;
        match result.status() {
            status if status.is_success() => Ok(()),
            StatusCode::TOO_MANY_REQUESTS
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT => Err(ExportError::Unavailable),
            _ => Err(ExportError::Rejected),
        }
    }
}

impl fmt::Debug for OtlpHttpExporter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OtlpHttpExporter")
            .field("traces_endpoint", &self.traces_endpoint)
            .field("logs_endpoint", &self.logs_endpoint)
            .field("authorization", &"[REDACTED]")
            .field("service_name", &self.service_name)
            .finish_non_exhaustive()
    }
}

impl OtelExporter for OtlpHttpExporter {
    fn export(&self, signal: OtelSignal) -> LocalBoxFuture<'static, Result<(), ExportError>> {
        let exporter = self.clone();
        Box::pin(async move {
            match signal {
                OtelSignal::Span(span) => {
                    let request = trace_request(&exporter.service_name, span)?;
                    exporter
                        .post(exporter.traces_endpoint.clone(), request.encode_to_vec())
                        .await
                }
                OtelSignal::Log(log) => {
                    let request = logs_request(&exporter.service_name, log);
                    exporter
                        .post(exporter.logs_endpoint.clone(), request.encode_to_vec())
                        .await
                }
                OtelSignal::Metric(_) => Err(ExportError::Rejected),
            }
        })
    }
}

fn trace_request(
    service_name: &str,
    signal: OtelSpan,
) -> Result<ExportTraceServiceRequest, ExportError> {
    let ended_at = signal.ended_at.ok_or(ExportError::Rejected)?;
    let elapsed = ended_at
        .checked_sub(signal.started_at)
        .ok_or(ExportError::Rejected)?;
    let end_time_unix_nano = unix_nanos();
    let start_time_unix_nano = end_time_unix_nano.saturating_sub(duration_nanos(elapsed));
    let kind = if signal.parent_span_id.is_none()
        && (signal.attributes.contains_key("http.request.method")
            || signal.attributes.contains_key("http.method"))
    {
        span::SpanKind::Server
    } else {
        span::SpanKind::Internal
    };
    let status = if signal.attributes.contains_key("error.type")
        || signal
            .attributes
            .get("http.response.status_code")
            .or_else(|| signal.attributes.get("http.status_code"))
            .and_then(|value| value.parse::<u16>().ok())
            .is_some_and(|status| status >= 500)
    {
        2
    } else {
        1
    };
    Ok(ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            resource: Some(resource(service_name)),
            scope_spans: vec![ScopeSpans {
                scope: Some(scope()),
                spans: vec![Span {
                    trace_id: signal.trace_context.trace_id().to_vec(),
                    span_id: signal.trace_context.span_id().to_vec(),
                    trace_state: signal
                        .trace_context
                        .tracestate()
                        .unwrap_or_default()
                        .to_owned(),
                    parent_span_id: signal
                        .parent_span_id
                        .map_or_else(Vec::new, |parent| parent.to_vec()),
                    name: signal.name,
                    kind: kind.into(),
                    start_time_unix_nano,
                    end_time_unix_nano,
                    attributes: attributes(signal.attributes),
                    flags: u32::from(signal.trace_context.trace_flags()),
                    status: Some(Status {
                        message: String::new(),
                        code: status,
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
    })
}

fn logs_request(service_name: &str, signal: OtelLog) -> ExportLogsServiceRequest {
    ExportLogsServiceRequest {
        resource_logs: vec![ResourceLogs {
            resource: Some(resource(service_name)),
            scope_logs: vec![ScopeLogs {
                scope: Some(scope()),
                log_records: vec![LogRecord {
                    time_unix_nano: unix_nanos(),
                    observed_time_unix_nano: unix_nanos(),
                    severity_number: match signal.severity {
                        OtelSeverity::Info => 9,
                        OtelSeverity::Warn => 13,
                        OtelSeverity::Error => 17,
                    },
                    severity_text: match signal.severity {
                        OtelSeverity::Info => "INFO",
                        OtelSeverity::Warn => "WARN",
                        OtelSeverity::Error => "ERROR",
                    }
                    .to_owned(),
                    body: Some(string_value(signal.body)),
                    attributes: attributes(signal.attributes),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
}

fn resource(service_name: &str) -> Resource {
    Resource {
        attributes: vec![KeyValue {
            key: "service.name".to_owned(),
            value: Some(string_value(service_name.to_owned())),
            key_strindex: 0,
        }],
        ..Default::default()
    }
}

fn scope() -> InstrumentationScope {
    InstrumentationScope {
        name: INSTRUMENTATION_SCOPE_NAME.to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        ..Default::default()
    }
}

fn attributes(values: std::collections::BTreeMap<String, String>) -> Vec<KeyValue> {
    values
        .into_iter()
        .map(|(key, value)| KeyValue {
            key,
            value: Some(string_value(value)),
            key_strindex: 0,
        })
        .collect()
}

fn string_value(value: String) -> AnyValue {
    AnyValue {
        value: Some(any_value::Value::StringValue(value)),
    }
}

fn unix_nanos() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, duration_nanos)
}

fn duration_nanos(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}
