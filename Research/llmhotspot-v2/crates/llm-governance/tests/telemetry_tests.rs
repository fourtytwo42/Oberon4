use llm_governance::{TelemetryEvent, TelemetrySeverity};

#[test]
fn telemetry_event_schema_roundtrip() {
    let evt = TelemetryEvent {
        request_id: "r1".to_string(),
        metric_name: "auth_oauth_start".to_string(),
        severity: TelemetrySeverity::Info,
        detail: "flow started".to_string(),
    };

    let raw = serde_json::to_string(&evt).expect("serialize");
    let parsed: TelemetryEvent = serde_json::from_str(&raw).expect("parse");
    assert_eq!(parsed, evt);
}
