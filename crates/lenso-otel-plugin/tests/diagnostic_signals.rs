use std::{collections::BTreeMap, time::Duration};

use lenso_kernel::{
    DiagnosticEvent, DiagnosticOutcome, DiagnosticRecord, DiagnosticSource, RuntimeFailureKind,
};
use lenso_otel_plugin::{OtelSeverity, OtelSignal, diagnostic_to_signal};

#[test]
fn converts_structural_runtime_facts_to_an_otel_log_without_payloads() {
    let record = DiagnosticRecord {
        sequence: 17,
        timestamp: Duration::from_millis(12),
        source: DiagnosticSource::Invocation,
        event: DiagnosticEvent::InvocationCompleted {
            requirement_id: Some("business".to_owned()),
            request_id: 42,
            caller_instance: Some("consumer".to_owned()),
            provider_instance: Some("provider".to_owned()),
            capability: "example.greeting@1",
            operation: Some("greet"),
            outcome: DiagnosticOutcome::DomainError,
            elapsed: Duration::from_micros(9),
        },
    };

    let OtelSignal::Log(log) = diagnostic_to_signal(&record) else {
        panic!("runtime diagnostics should export as structural OTel logs");
    };
    assert_eq!(log.severity, OtelSeverity::Info);
    assert_eq!(log.body, "lenso.runtime.diagnostic");
    assert_eq!(log.timestamp, Duration::from_millis(12));
    assert_eq!(
        log.attributes.get("lenso.diagnostic.sequence"),
        Some(&"17".to_owned())
    );
    assert_eq!(
        log.attributes.get("lenso.diagnostic.outcome"),
        Some(&"domain_error".to_owned())
    );
    assert_eq!(
        log.attributes.get("lenso.requirement.id"),
        Some(&"business".to_owned())
    );
    assert_eq!(
        log.attributes.get("lenso.request.id"),
        Some(&"42".to_owned())
    );
    assert!(!format!("{log:?}").contains("secret"));
}

#[test]
fn runtime_failure_export_contains_only_a_sanitized_category() {
    let record = DiagnosticRecord {
        sequence: 18,
        timestamp: Duration::ZERO,
        source: DiagnosticSource::RuntimeFailure,
        event: DiagnosticEvent::RuntimeFailure {
            instance: Some("provider".to_owned()),
            kind: RuntimeFailureKind::PluginFailure,
        },
    };

    let OtelSignal::Log(log) = diagnostic_to_signal(&record) else {
        panic!("runtime failures should export as structural OTel logs");
    };
    assert_eq!(log.severity, OtelSeverity::Error);
    assert_eq!(
        log.attributes.get("lenso.runtime_failure.kind"),
        Some(&"plugin_failure".to_owned())
    );
    assert_eq!(log.attributes.len(), 6);
}

#[test]
fn application_signals_have_explicit_otel_shapes() {
    let span = lenso_otel_plugin::OtelSpan {
        name: "greet".to_owned(),
        trace_context: lenso_otel_plugin::TraceContext::from_traceparent(
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            None,
        )
        .expect("trace context is valid"),
        parent_span_id: None,
        started_at: Duration::ZERO,
        ended_at: Some(Duration::from_millis(1)),
        attributes: BTreeMap::new(),
    };
    let metric = lenso_otel_plugin::OtelMetric {
        name: "requests".to_owned(),
        value: 1.0,
        unit: Some("{request}".to_owned()),
        timestamp: Duration::ZERO,
        attributes: BTreeMap::new(),
    };

    assert!(matches!(OtelSignal::Span(span), OtelSignal::Span(_)));
    assert!(matches!(OtelSignal::Metric(metric), OtelSignal::Metric(_)));
}
