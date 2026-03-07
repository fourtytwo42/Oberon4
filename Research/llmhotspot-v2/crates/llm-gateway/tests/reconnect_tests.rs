use llm_gateway::BackoffState;

#[test]
fn backoff_increases() {
    let b0 = BackoffState::reset();
    let b1 = b0.increment();
    let b2 = b1.increment();

    assert!(b1.next_delay_ms() > b0.next_delay_ms());
    assert!(b2.next_delay_ms() > b1.next_delay_ms());
}
