use std::{collections::BTreeMap, time::Duration};

use lenso_kernel::{
    DiagnosticAdmission, DiagnosticEvent, DiagnosticOutcome, DiagnosticRecord,
    DiagnosticShutdownOutcome, DiagnosticSource, PluginLifecyclePhase, RuntimeFailureKind,
};

use crate::TraceContext;

/// `OTel` severity assigned to one exported signal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OtelSeverity {
    /// Informational runtime or application signal.
    Info,
    /// A lossy runtime condition worth operator attention.
    Warn,
    /// A sanitized Runtime Failure category.
    Error,
}

/// An explicit application span signal.
#[derive(Clone, Debug, PartialEq)]
pub struct OtelSpan {
    /// Instrumentation operation name.
    pub name: String,
    /// W3C trace and current span identity.
    pub trace_context: TraceContext,
    /// Parent span identity when the application created a child span.
    pub parent_span_id: Option<[u8; 8]>,
    /// Driver-monotonic start instant supplied by the application or Adapter.
    pub started_at: Duration,
    /// Driver-monotonic completion instant, when the span has ended.
    pub ended_at: Option<Duration>,
    /// Explicit application attributes; Runtime Diagnostics never populate this map from payloads.
    pub attributes: BTreeMap<String, String>,
}

/// An explicit application metric signal.
#[derive(Clone, Debug, PartialEq)]
pub struct OtelMetric {
    /// Metric identity.
    pub name: String,
    /// Numeric measurement.
    pub value: f64,
    /// Optional OpenTelemetry unit.
    pub unit: Option<String>,
    /// Driver-monotonic measurement instant.
    pub timestamp: Duration,
    /// Explicit application attributes.
    pub attributes: BTreeMap<String, String>,
}

/// A structural Runtime Diagnostic or explicit application log signal.
#[derive(Clone, Debug, PartialEq)]
pub struct OtelLog {
    /// Driver-monotonic observation instant.
    pub timestamp: Duration,
    /// Normalized severity.
    pub severity: OtelSeverity,
    /// Stable log body or event name.
    pub body: String,
    /// Sanitized structural or explicitly supplied attributes.
    pub attributes: BTreeMap<String, String>,
}

/// Signal accepted by an `OTel` Module exporter.
#[derive(Clone, Debug, PartialEq)]
pub enum OtelSignal {
    /// A span created by application instrumentation.
    Span(OtelSpan),
    /// A metric created by application instrumentation.
    Metric(OtelMetric),
    /// A Runtime Diagnostic or explicit application log.
    Log(OtelLog),
}

/// Converts one sanitized Kernel Runtime Diagnostic into a structural `OTel` Log.
///
/// The conversion deliberately enumerates every allowed Diagnostic Event field.
/// It never formats or serializes the source record, so payloads, configuration,
/// secrets, `ActorAssertions`, and arbitrary extensions cannot
/// leak into an exported signal.
#[allow(clippy::too_many_lines)]
pub fn diagnostic_to_signal(record: &DiagnosticRecord) -> OtelSignal {
    let mut attributes = BTreeMap::new();
    attribute(
        &mut attributes,
        "lenso.diagnostic.sequence",
        record.sequence,
    );
    attribute(
        &mut attributes,
        "lenso.diagnostic.timestamp_nanos",
        record.timestamp.as_nanos(),
    );
    attribute(
        &mut attributes,
        "lenso.diagnostic.source",
        diagnostic_source_name(record.source),
    );
    attribute(
        &mut attributes,
        "lenso.diagnostic.event",
        diagnostic_event_name(&record.event),
    );

    let severity = match &record.event {
        DiagnosticEvent::RuntimeFailure { .. }
        | DiagnosticEvent::RestartExhausted { terminal: true, .. } => OtelSeverity::Error,
        DiagnosticEvent::AdmissionRejected {
            outcome: DiagnosticAdmission::Exhausted | DiagnosticAdmission::Closed,
            ..
        } => OtelSeverity::Warn,
        _ => OtelSeverity::Info,
    };

    match &record.event {
        DiagnosticEvent::AppStarted { plugin_count } => {
            attribute(&mut attributes, "lenso.app.plugin_count", *plugin_count);
        }
        DiagnosticEvent::AppReady
        | DiagnosticEvent::ShutdownAdmissionClosed
        | DiagnosticEvent::ShutdownCleanupStarted { .. } => {}
        DiagnosticEvent::LifecycleStarted {
            instance,
            generation,
            phase,
        } => {
            attribute(&mut attributes, "lenso.plugin.instance", instance);
            attribute(&mut attributes, "lenso.plugin.generation", *generation);
            attribute(
                &mut attributes,
                "lenso.plugin.phase",
                lifecycle_phase_name(*phase),
            );
        }
        DiagnosticEvent::LifecycleCompleted {
            instance,
            generation,
            phase,
            outcome,
            elapsed,
        } => {
            attribute(&mut attributes, "lenso.plugin.instance", instance);
            attribute(&mut attributes, "lenso.plugin.generation", *generation);
            attribute(
                &mut attributes,
                "lenso.plugin.phase",
                lifecycle_phase_name(*phase),
            );
            attribute(
                &mut attributes,
                "lenso.diagnostic.outcome",
                outcome_name(*outcome),
            );
            attribute(
                &mut attributes,
                "lenso.diagnostic.elapsed_nanos",
                elapsed.as_nanos(),
            );
        }
        DiagnosticEvent::InvocationStarted {
            requirement_id,
            request_id,
            caller_instance,
            provider_instance,
            capability,
            operation,
        } => {
            invocation_attributes(
                &mut attributes,
                requirement_id.as_deref(),
                *request_id,
                caller_instance.as_deref(),
                provider_instance.as_deref(),
                capability,
                *operation,
            );
        }
        DiagnosticEvent::InvocationCompleted {
            requirement_id,
            request_id,
            caller_instance,
            provider_instance,
            capability,
            operation,
            outcome,
            elapsed,
        } => {
            invocation_attributes(
                &mut attributes,
                requirement_id.as_deref(),
                *request_id,
                caller_instance.as_deref(),
                provider_instance.as_deref(),
                capability,
                *operation,
            );
            attribute(
                &mut attributes,
                "lenso.diagnostic.outcome",
                outcome_name(*outcome),
            );
            attribute(
                &mut attributes,
                "lenso.diagnostic.elapsed_nanos",
                elapsed.as_nanos(),
            );
        }
        DiagnosticEvent::AdmissionRejected {
            requirement_id,
            request_id,
            caller_instance,
            provider_instance,
            capability,
            operation,
            outcome,
        } => {
            invocation_attributes(
                &mut attributes,
                requirement_id.as_deref(),
                *request_id,
                caller_instance.as_deref(),
                provider_instance.as_deref(),
                capability,
                *operation,
            );
            attribute(
                &mut attributes,
                "lenso.admission.outcome",
                admission_name(*outcome),
            );
        }
        DiagnosticEvent::EventAdmission {
            requirement_id,
            request_id,
            publisher_instance,
            subscriber_instance,
            capability,
            operation,
            outcome,
        } => {
            optional_attribute(
                &mut attributes,
                "lenso.requirement.id",
                requirement_id.as_deref(),
            );
            attribute(&mut attributes, "lenso.request.id", *request_id);
            attribute(
                &mut attributes,
                "lenso.event.publisher_instance",
                publisher_instance,
            );
            attribute(
                &mut attributes,
                "lenso.event.subscriber_instance",
                subscriber_instance,
            );
            attribute(&mut attributes, "lenso.capability.id", capability);
            optional_attribute(&mut attributes, "lenso.operation.name", *operation);
            attribute(
                &mut attributes,
                "lenso.admission.outcome",
                admission_name(*outcome),
            );
        }
        DiagnosticEvent::GenerationUnavailable {
            instance,
            generation,
        }
        | DiagnosticEvent::GenerationReady {
            instance,
            generation,
        } => {
            attribute(&mut attributes, "lenso.plugin.instance", instance);
            attribute(&mut attributes, "lenso.plugin.generation", *generation);
        }
        DiagnosticEvent::RestartScheduled {
            instance,
            attempt,
            delay,
        } => {
            attribute(&mut attributes, "lenso.plugin.instance", instance);
            attribute(&mut attributes, "lenso.supervision.attempt", *attempt);
            attribute(
                &mut attributes,
                "lenso.supervision.delay_nanos",
                delay.as_nanos(),
            );
        }
        DiagnosticEvent::RestartExhausted {
            instance,
            attempts,
            terminal,
        } => {
            attribute(&mut attributes, "lenso.plugin.instance", instance);
            attribute(&mut attributes, "lenso.supervision.attempts", *attempts);
            attribute(&mut attributes, "lenso.supervision.terminal", *terminal);
        }
        DiagnosticEvent::RuntimeFailure { instance, kind } => {
            optional_attribute(
                &mut attributes,
                "lenso.plugin.instance",
                instance.as_deref(),
            );
            attribute(
                &mut attributes,
                "lenso.runtime_failure.kind",
                runtime_failure_name(*kind),
            );
        }
        DiagnosticEvent::ShutdownCompleted { outcome, elapsed } => {
            attribute(
                &mut attributes,
                "lenso.shutdown.outcome",
                shutdown_outcome_name(*outcome),
            );
            attribute(
                &mut attributes,
                "lenso.diagnostic.elapsed_nanos",
                elapsed.as_nanos(),
            );
        }
    }

    OtelSignal::Log(OtelLog {
        timestamp: record.timestamp,
        severity,
        body: "lenso.runtime.diagnostic".to_owned(),
        attributes,
    })
}

fn invocation_attributes(
    attributes: &mut BTreeMap<String, String>,
    requirement_id: Option<&str>,
    request_id: u64,
    caller_instance: Option<&str>,
    provider_instance: Option<&str>,
    capability: &'static str,
    operation: Option<&'static str>,
) {
    optional_attribute(attributes, "lenso.requirement.id", requirement_id);
    attribute(attributes, "lenso.request.id", request_id);
    optional_attribute(attributes, "lenso.caller.instance", caller_instance);
    optional_attribute(attributes, "lenso.provider.instance", provider_instance);
    attribute(attributes, "lenso.capability.id", capability);
    optional_attribute(attributes, "lenso.operation.name", operation);
}

#[allow(clippy::needless_pass_by_value)]
fn attribute<T: ToString>(attributes: &mut BTreeMap<String, String>, key: &str, value: T) {
    attributes.insert(key.to_owned(), value.to_string());
}

fn optional_attribute(attributes: &mut BTreeMap<String, String>, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        attribute(attributes, key, value);
    }
}

fn diagnostic_source_name(source: DiagnosticSource) -> &'static str {
    match source {
        DiagnosticSource::Lifecycle => "lifecycle",
        DiagnosticSource::Invocation => "invocation",
        DiagnosticSource::Admission => "admission",
        DiagnosticSource::Supervision => "supervision",
        DiagnosticSource::Shutdown => "shutdown",
        DiagnosticSource::RuntimeFailure => "runtime_failure",
    }
}

fn diagnostic_event_name(event: &DiagnosticEvent) -> &'static str {
    match event {
        DiagnosticEvent::AppStarted { .. } => "app_started",
        DiagnosticEvent::AppReady => "app_ready",
        DiagnosticEvent::LifecycleStarted { .. } => "lifecycle_started",
        DiagnosticEvent::LifecycleCompleted { .. } => "lifecycle_completed",
        DiagnosticEvent::InvocationStarted { .. } => "invocation_started",
        DiagnosticEvent::InvocationCompleted { .. } => "invocation_completed",
        DiagnosticEvent::AdmissionRejected { .. } => "admission_rejected",
        DiagnosticEvent::EventAdmission { .. } => "event_admission",
        DiagnosticEvent::GenerationUnavailable { .. } => "generation_unavailable",
        DiagnosticEvent::GenerationReady { .. } => "generation_ready",
        DiagnosticEvent::RestartScheduled { .. } => "restart_scheduled",
        DiagnosticEvent::RestartExhausted { .. } => "restart_exhausted",
        DiagnosticEvent::RuntimeFailure { .. } => "runtime_failure",
        DiagnosticEvent::ShutdownAdmissionClosed => "shutdown_admission_closed",
        DiagnosticEvent::ShutdownCleanupStarted { .. } => "shutdown_cleanup_started",
        DiagnosticEvent::ShutdownCompleted { .. } => "shutdown_completed",
    }
}

fn lifecycle_phase_name(phase: PluginLifecyclePhase) -> &'static str {
    match phase {
        PluginLifecyclePhase::Construct => "construct",
        PluginLifecyclePhase::Prepare => "prepare",
        PluginLifecyclePhase::Activate => "activate",
        PluginLifecyclePhase::Ready => "ready",
        PluginLifecyclePhase::Deactivate => "deactivate",
    }
}

fn outcome_name(outcome: DiagnosticOutcome) -> &'static str {
    match outcome {
        DiagnosticOutcome::Succeeded => "succeeded",
        DiagnosticOutcome::DomainError => "domain_error",
        DiagnosticOutcome::RuntimeFailure(_) => "runtime_failure",
    }
}

fn admission_name(outcome: DiagnosticAdmission) -> &'static str {
    match outcome {
        DiagnosticAdmission::Accepted => "accepted",
        DiagnosticAdmission::Unavailable => "unavailable",
        DiagnosticAdmission::Exhausted => "exhausted",
        DiagnosticAdmission::Closed => "closed",
    }
}

fn shutdown_outcome_name(outcome: DiagnosticShutdownOutcome) -> &'static str {
    match outcome {
        DiagnosticShutdownOutcome::Clean => "clean",
        DiagnosticShutdownOutcome::RuntimeFailure => "runtime_failure",
        DiagnosticShutdownOutcome::Timeout => "timeout",
    }
}

fn runtime_failure_name(kind: RuntimeFailureKind) -> &'static str {
    match kind {
        RuntimeFailureKind::Unavailable => "unavailable",
        RuntimeFailureKind::UnknownOperation => "unknown_operation",
        RuntimeFailureKind::AmbiguousBinding => "ambiguous_binding",
        RuntimeFailureKind::ProtocolViolation => "protocol_violation",
        RuntimeFailureKind::MissingPluginFactory => "missing_plugin_factory",
        RuntimeFailureKind::UnavailableExecutionClass => "unavailable_execution_class",
        RuntimeFailureKind::InvalidResolvedPlan => "invalid_resolved_plan",
        RuntimeFailureKind::AdmissionClosed => "admission_closed",
        RuntimeFailureKind::ResourceExhausted => "resource_exhausted",
        RuntimeFailureKind::DeadlineExceeded => "deadline_exceeded",
        RuntimeFailureKind::Cancelled => "cancelled",
        RuntimeFailureKind::Internal => "internal",
        RuntimeFailureKind::PluginFailure => "plugin_failure",
        RuntimeFailureKind::PluginRestartExhausted => "plugin_restart_exhausted",
    }
}
