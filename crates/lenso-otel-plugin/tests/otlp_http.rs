use std::{collections::BTreeMap, time::Duration};

use lenso_otel_plugin::{
    OtelExporter, OtelLog, OtelSeverity, OtelSignal, OtelSpan, OtlpHttpExporter,
    OtlpHttpExporterConfigError, TraceContext,
};
use opentelemetry_proto::tonic::{
    collector::{logs::v1::ExportLogsServiceRequest, trace::v1::ExportTraceServiceRequest},
    trace::v1::span,
};
use prost::Message as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

#[tokio::test(flavor = "current_thread")]
async fn exports_a_completed_http_server_span_as_otlp_protobuf() {
    let mut attributes = BTreeMap::new();
    attributes.insert("http.request.method".to_owned(), "GET".to_owned());
    attributes.insert("http.route".to_owned(), "/orders/:id".to_owned());
    attributes.insert("http.response.status_code".to_owned(), "200".to_owned());
    let signal = OtelSignal::Span(OtelSpan {
        name: "GET /orders/:id".to_owned(),
        trace_context: trace_context(),
        parent_span_id: None,
        started_at: Duration::from_secs(10),
        ended_at: Some(Duration::from_secs(10) + Duration::from_millis(25)),
        attributes,
    });

    let request = capture(signal).await;

    assert_eq!(request.path, "/v1/traces");
    assert!(
        request
            .headers
            .contains("authorization: Bearer private-token")
    );
    assert!(
        request
            .headers
            .contains("content-type: application/x-protobuf")
    );
    let payload = ExportTraceServiceRequest::decode(request.body.as_slice()).unwrap();
    let resource_spans = &payload.resource_spans[0];
    assert_eq!(
        resource_spans.resource.as_ref().unwrap().attributes[0]
            .value
            .as_ref()
            .unwrap()
            .value,
        Some(
            opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(
                "sample-web".to_owned()
            )
        )
    );
    let span = &resource_spans.scope_spans[0].spans[0];
    assert_eq!(span.name, "GET /orders/:id");
    assert_eq!(span.kind, i32::from(span::SpanKind::Server));
    assert_eq!(
        span.end_time_unix_nano - span.start_time_unix_nano,
        25_000_000
    );
    assert_eq!(span.trace_id, vec![1; 16]);
    assert_eq!(span.span_id, vec![2; 8]);
}

#[tokio::test(flavor = "current_thread")]
async fn exports_logs_to_the_distinct_otlp_endpoint() {
    let signal = OtelSignal::Log(OtelLog {
        timestamp: Duration::from_secs(1),
        severity: OtelSeverity::Warn,
        body: "sample warning".to_owned(),
        attributes: BTreeMap::from([("code.function.name".to_owned(), "serve".to_owned())]),
    });

    let request = capture(signal).await;

    assert_eq!(request.path, "/v1/logs");
    let payload = ExportLogsServiceRequest::decode(request.body.as_slice()).unwrap();
    let record = &payload.resource_logs[0].scope_logs[0].log_records[0];
    assert_eq!(record.severity_text, "WARN");
    assert_eq!(record.attributes[0].key, "code.function.name");
}

#[test]
fn validates_private_exporter_configuration_without_exposing_the_token() {
    assert!(matches!(
        OtlpHttpExporter::new("file:///tmp/collector", "token", "service"),
        Err(OtlpHttpExporterConfigError::InvalidEndpoint)
    ));
    assert!(matches!(
        OtlpHttpExporter::new("http://127.0.0.1:4318", "", "service"),
        Err(OtlpHttpExporterConfigError::InvalidBearerToken)
    ));
    let exporter = OtlpHttpExporter::new(
        "http://127.0.0.1:4318/otel",
        "do-not-print-this-token",
        "service",
    )
    .unwrap();
    assert!(!format!("{exporter:?}").contains("do-not-print-this-token"));
}

fn trace_context() -> TraceContext {
    TraceContext::from_traceparent(
        "00-01010101010101010101010101010101-0202020202020202-01",
        None,
    )
    .unwrap()
}

#[derive(Debug)]
struct CapturedRequest {
    path: String,
    headers: String,
    body: Vec<u8>,
}

async fn capture(signal: OtelSignal) -> CapturedRequest {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 4096];
        let header_end = loop {
            let read = socket.read(&mut chunk).await.unwrap();
            assert!(read > 0);
            bytes.extend_from_slice(&chunk[..read]);
            if let Some(position) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
                break position + 4;
            }
        };
        let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(str::trim)
                    .and_then(|value| value.parse::<usize>().ok())
            })
            .unwrap();
        while bytes.len() - header_end < content_length {
            let read = socket.read(&mut chunk).await.unwrap();
            assert!(read > 0);
            bytes.extend_from_slice(&chunk[..read]);
        }
        socket
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
            .await
            .unwrap();
        let path = headers
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .to_owned();
        CapturedRequest {
            path,
            headers,
            body: bytes[header_end..header_end + content_length].to_vec(),
        }
    });
    let exporter =
        OtlpHttpExporter::new(&format!("http://{address}"), "private-token", "sample-web").unwrap();
    exporter.export(signal).await.unwrap();
    server.await.unwrap()
}
