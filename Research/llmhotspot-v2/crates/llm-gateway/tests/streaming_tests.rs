use llm_gateway::{assemble_chunks, RelayFrame};

#[test]
fn stream_chunk_integrity() {
    let frames = vec![
        RelayFrame::ResponseChunk {
            request_id: "r1".to_string(),
            chunk: "hel".to_string(),
        },
        RelayFrame::ResponseChunk {
            request_id: "r1".to_string(),
            chunk: "lo".to_string(),
        },
        RelayFrame::ResponseEnd {
            request_id: "r1".to_string(),
        },
    ];

    let out = assemble_chunks(&frames, "r1").expect("assembled");
    assert_eq!(out, "hello");
}
