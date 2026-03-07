use llm_foundation::runtime::{now_unix, RuntimeInfo};

#[test]
fn runtime_info_has_values() {
    let rt = RuntimeInfo::new("llm-control-api", "0.1.0");
    assert_eq!(rt.service_name, "llm-control-api");
    assert_eq!(rt.version, "0.1.0");
    assert!(rt.started_unix > 0);
}

#[test]
fn now_unix_monotonic_enough() {
    let a = now_unix();
    let b = now_unix();
    assert!(b >= a);
}
