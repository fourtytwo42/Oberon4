use axum::{body::Body, http::StatusCode, response::IntoResponse};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::time::{Duration, MissedTickBehavior};

use super::{now_unix, sse_keepalive_ms};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedSseFrame {
    pub(crate) event: Option<String>,
    pub(crate) data: String,
}

#[derive(Debug, Default)]
pub(crate) struct SseFrameParser {
    event: Option<String>,
    data_lines: Vec<String>,
}

impl SseFrameParser {
    pub(crate) fn push_line(&mut self, raw: &str) -> Option<ParsedSseFrame> {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() {
            return self.take_frame();
        }
        // Comments / keepalive pings
        if line.starts_with(':') {
            return None;
        }
        if let Some(v) = line.strip_prefix("event:") {
            self.event = Some(v.trim().to_string());
            return None;
        }
        if let Some(v) = line.strip_prefix("data:") {
            // Per SSE spec, the optional single leading space after "data:" is ignored.
            self.data_lines.push(v.trim_start().to_string());
            return None;
        }
        None
    }

    fn take_frame(&mut self) -> Option<ParsedSseFrame> {
        if self.event.is_none() && self.data_lines.is_empty() {
            return None;
        }
        let data = self.data_lines.join("\n");
        let frame = ParsedSseFrame {
            event: self.event.take(),
            data,
        };
        self.data_lines.clear();
        Some(frame)
    }

    pub(crate) fn flush(&mut self) -> Option<ParsedSseFrame> {
        self.take_frame()
    }
}

#[derive(Debug, Default)]
pub(crate) struct ResponsesStreamState {
    created_unix: u64,
    response_id: String,
    model: String,
    seq: u64,
    created_emitted: bool,
    completed_emitted: bool,
    tool_item_id_by_call_id: HashMap<String, String>,
    tool_name_by_item_id: HashMap<String, String>,
    tool_args_accumulated: HashMap<String, String>,
    tool_args_seen_delta: HashMap<String, bool>,
    next_tool_idx: u64,
}

impl ResponsesStreamState {
    fn new() -> Self {
        let created_unix = now_unix();
        Self {
            created_unix,
            response_id: format!("resp_llmhotspot_{created_unix}"),
            model: "gpt-5.2".to_string(),
            seq: 0,
            created_emitted: false,
            completed_emitted: false,
            tool_item_id_by_call_id: HashMap::new(),
            tool_name_by_item_id: HashMap::new(),
            tool_args_accumulated: HashMap::new(),
            tool_args_seen_delta: HashMap::new(),
            next_tool_idx: 0,
        }
    }

    fn next_seq(&mut self) -> u64 {
        self.seq = self.seq.saturating_add(1);
        self.seq
    }

    fn ensure_created_events(&mut self) -> Vec<(String, Value)> {
        if self.created_emitted {
            return vec![];
        }
        self.created_emitted = true;
        vec![(
            "response.created".to_string(),
            json!({
                "type": "response.created",
                "response": {
                    "id": self.response_id,
                    "model": self.model,
                    "status": "in_progress",
                    "created": self.created_unix as i64,
                }
            }),
        )]
    }

    fn finalize_events(&mut self) -> Vec<(String, Value)> {
        if self.completed_emitted {
            return vec![];
        }
        self.completed_emitted = true;
        let mut out: Vec<(String, Value)> = Vec::new();

        // Emit output_item.done for any tool calls we announced.
        for (item_id, name) in self.tool_name_by_item_id.clone().into_iter() {
            let args = self
                .tool_args_accumulated
                .get(&item_id)
                .cloned()
                .unwrap_or_default();
            let saw_delta = self
                .tool_args_seen_delta
                .get(&item_id)
                .copied()
                .unwrap_or(false);
            let mut item = json!({
                "type": "function_call",
                "id": item_id,
                "name": name,
            });
            if !saw_delta && !args.is_empty() {
                item["arguments"] = Value::String(args);
            }
            out.push((
                "response.output_item.done".to_string(),
                json!({
                    "type": "response.output_item.done",
                    "item": item
                }),
            ));
        }

        out.push((
            "response.completed".to_string(),
            json!({
                "type": "response.completed",
                "response": {
                    "id": self.response_id,
                    "model": self.model,
                    "status": "completed",
                    "created": self.created_unix as i64,
                }
            }),
        ));
        out
    }

    fn tool_item_id_for_call_id(&mut self, call_id: &str) -> String {
        if let Some(v) = self.tool_item_id_by_call_id.get(call_id) {
            return v.clone();
        }
        self.next_tool_idx = self.next_tool_idx.saturating_add(1);
        let item_id = if call_id.is_empty() {
            format!("tool_item_{}", self.next_tool_idx)
        } else {
            format!("tool_item_{}", call_id)
        };
        self.tool_item_id_by_call_id
            .insert(call_id.to_string(), item_id.clone());
        item_id
    }
}

pub(crate) fn ensure_type_and_seq(event_name: &str, seq: u64, payload: &mut Value) {
    if payload.get("type").and_then(Value::as_str) != Some(event_name) {
        payload["type"] = Value::String(event_name.to_string());
    }
    if payload.get("sequence_number").is_none() {
        payload["sequence_number"] = json!(seq);
    }
}

pub(crate) fn chat_chunk_to_responses_events(
    state: &mut ResponsesStreamState,
    chunk: &Value,
) -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = Vec::new();
    if let Some(m) = chunk.get("model").and_then(Value::as_str) {
        if !m.trim().is_empty() {
            state.model = m.to_string();
        }
    }

    out.extend(state.ensure_created_events());

    let choices = chunk
        .get("choices")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let Some(first) = choices.first() else {
        return out;
    };

    // Text deltas
    if let Some(delta_text) = first
        .get("delta")
        .and_then(|d| d.get("content"))
        .and_then(Value::as_str)
    {
        if !delta_text.is_empty() {
            out.push((
                "response.output_text.delta".to_string(),
                json!({
                    "type": "response.output_text.delta",
                    "delta": delta_text,
                }),
            ));
        }
    }

    // Tool calls
    if let Some(tool_calls) = first
        .get("delta")
        .and_then(|d| d.get("tool_calls"))
        .and_then(Value::as_array)
    {
        for tc in tool_calls {
            let call_id = tc.get("id").and_then(Value::as_str).unwrap_or("");
            let name = tc
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let args_delta = tc
                .get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(Value::as_str)
                .unwrap_or("");

            let item_id = state.tool_item_id_for_call_id(call_id);
            if !name.is_empty() && !state.tool_name_by_item_id.contains_key(&item_id) {
                state
                    .tool_name_by_item_id
                    .insert(item_id.clone(), name.to_string());
                out.push((
                    "response.output_item.added".to_string(),
                    json!({
                        "type": "response.output_item.added",
                        "item": {
                            "type": "function_call",
                            "id": item_id,
                            "call_id": call_id,
                            "name": name,
                        }
                    }),
                ));
            }

            if !args_delta.is_empty() {
                state
                    .tool_args_accumulated
                    .entry(item_id.clone())
                    .and_modify(|s| s.push_str(args_delta))
                    .or_insert_with(|| args_delta.to_string());
                state.tool_args_seen_delta.insert(item_id.clone(), true);
                out.push((
                    "response.function_call_arguments.delta".to_string(),
                    json!({
                        "type": "response.function_call_arguments.delta",
                        "item_id": item_id,
                        "delta": args_delta,
                    }),
                ));
            }
        }
    }

    // Completion hint from finish_reason
    if first.get("finish_reason").is_some() {
        out.extend(state.finalize_events());
    }

    out
}

pub(crate) fn responses_sse_normalized(upstream: reqwest::Response) -> axum::response::Response {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, std::io::Error>>(64);
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

    tokio::spawn(async move {
        let stop_pings = Arc::new(AtomicBool::new(false));
        let tx_ping = tx.clone();
        let stop_pings_bg = stop_pings.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(sse_keepalive_ms()));
            interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                if stop_pings_bg.load(Ordering::Relaxed) {
                    break;
                }
                if tx_ping.send(Ok(": ping\n\n".to_string())).await.is_err() {
                    break;
                }
            }
        });

        let mut parser = SseFrameParser::default();
        let mut buf = String::new();
        let mut resp = upstream;
        let mut done_seen = false;
        let mut state = ResponsesStreamState::new();

        async fn send_event(
            tx: &tokio::sync::mpsc::Sender<Result<String, std::io::Error>>,
            event_name: &str,
            seq: u64,
            mut payload: Value,
        ) {
            ensure_type_and_seq(event_name, seq, &mut payload);
            let data = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
            let s = format!("event: {event_name}\ndata: {data}\n\n");
            let _ = tx.send(Ok(s)).await;
        }

        while let Ok(Some(chunk)) = resp.chunk().await {
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].to_string();
                buf = buf[pos + 1..].to_string();
                if let Some(frame) = parser.push_line(&line) {
                    let data = frame.data.trim().to_string();
                    if data == "[DONE]" {
                        if !state.completed_emitted {
                            for (ev, payload) in state.finalize_events() {
                                let seq = state.next_seq();
                                send_event(&tx, &ev, seq, payload).await;
                            }
                        }
                        let _ = tx.send(Ok("data: [DONE]\n\n".to_string())).await;
                        stop_pings.store(true, Ordering::Relaxed);
                        return;
                    }

                    let parsed = serde_json::from_str::<Value>(&data);
                    let Ok(value) = parsed else {
                        let seq = state.next_seq();
                        send_event(
                            &tx,
                            "response.error",
                            seq,
                            json!({
                                "type": "response.error",
                                "message": data,
                            }),
                        )
                        .await;
                        continue;
                    };

                    // If it's already a Responses event, normalize framing and ensure `type`.
                    let event_name = frame.event.clone().or_else(|| {
                        value
                            .get("type")
                            .and_then(Value::as_str)
                            .map(|s| s.to_string())
                    });

                    if let Some(ev) = event_name {
                        // Heuristic: treat `response.*` as Responses events.
                        if ev.starts_with("response.") || ev == "error" || ev == "response.error" {
                            let seq = state.next_seq();
                            send_event(&tx, &ev, seq, value).await;
                            continue;
                        }
                    }

                    // Otherwise, try to treat it as a Chat Completions chunk and translate.
                    let is_chat_chunk = value
                        .get("object")
                        .and_then(Value::as_str)
                        .map(|s| s == "chat.completion.chunk")
                        .unwrap_or(false);
                    if is_chat_chunk {
                        let events = chat_chunk_to_responses_events(&mut state, &value);
                        for (ev, payload) in events {
                            let seq = state.next_seq();
                            send_event(&tx, &ev, seq, payload).await;
                        }
                        continue;
                    }

                    let seq = state.next_seq();
                    send_event(
                        &tx,
                        "response.error",
                        seq,
                        json!({
                            "type": "response.error",
                            "message": "unrecognized upstream SSE payload",
                            "raw": value,
                        }),
                    )
                    .await;
                }
            }
        }

        // Flush any trailing partial frame.
        if let Some(frame) = parser.flush() {
            let data = frame.data.trim().to_string();
            if data == "[DONE]" {
                done_seen = true;
            }
        }

        if !done_seen {
            if !state.completed_emitted {
                for (ev, payload) in state.finalize_events() {
                    let seq = state.next_seq();
                    send_event(&tx, &ev, seq, payload).await;
                }
            }
            let _ = tx.send(Ok("data: [DONE]\n\n".to_string())).await;
        }
        stop_pings.store(true, Ordering::Relaxed);
    });

    let body = Body::from_stream(stream);
    let mut resp = (StatusCode::OK, body).into_response();
    resp.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        "text/event-stream".parse().expect("valid header"),
    );
    resp.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        "no-cache".parse().expect("valid header"),
    );
    resp
}

#[cfg(test)]
mod responses_sse_alignment_tests {
    use super::{
        chat_chunk_to_responses_events, ensure_type_and_seq, sse_keepalive_ms, ParsedSseFrame,
        ResponsesStreamState, SseFrameParser,
    };
    use serde_json::json;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn sse_keepalive_default_and_env() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::remove_var("LLMHOTSPOT_SSE_KEEPALIVE_MS");
        assert_eq!(sse_keepalive_ms(), 15_000);
        std::env::set_var("LLMHOTSPOT_SSE_KEEPALIVE_MS", "5000");
        assert_eq!(sse_keepalive_ms(), 5_000);
        std::env::remove_var("LLMHOTSPOT_SSE_KEEPALIVE_MS");
    }

    #[test]
    fn sse_parser_reads_event_and_multiline_data() {
        let mut p = SseFrameParser::default();
        assert_eq!(p.push_line("event: response.output_text.delta"), None);
        assert_eq!(p.push_line("data: {\"delta\":\"hi\"}"), None);
        assert_eq!(p.push_line("data: {\"more\":true}"), None);
        let frame = p.push_line("").expect("frame");
        assert_eq!(
            frame,
            ParsedSseFrame {
                event: Some("response.output_text.delta".to_string()),
                data: "{\"delta\":\"hi\"}\n{\"more\":true}".to_string()
            }
        );
    }

    #[test]
    fn ensure_type_and_seq_inserts_missing_fields() {
        let mut v = json!({"delta":"hi"});
        ensure_type_and_seq("response.output_text.delta", 7, &mut v);
        assert_eq!(
            v.get("type").and_then(|x| x.as_str()),
            Some("response.output_text.delta")
        );
        assert_eq!(v.get("sequence_number").and_then(|x| x.as_u64()), Some(7));
    }

    #[test]
    fn chat_chunk_translation_emits_created_then_text_delta() {
        let mut st = ResponsesStreamState::new();
        let chunk = json!({
            "object":"chat.completion.chunk",
            "model":"gpt-5.2",
            "choices":[{"delta":{"content":"hi"}}]
        });
        let events = chat_chunk_to_responses_events(&mut st, &chunk);
        assert!(!events.is_empty());
        assert_eq!(events[0].0, "response.created");
        assert_eq!(events[1].0, "response.output_text.delta");
        assert_eq!(
            events[1].1.get("delta").and_then(|x| x.as_str()),
            Some("hi")
        );
    }

    #[test]
    fn chat_chunk_translation_emits_tool_events() {
        let mut st = ResponsesStreamState::new();
        let chunk = json!({
            "object":"chat.completion.chunk",
            "model":"gpt-5.2",
            "choices":[{"delta":{"tool_calls":[{"id":"call_1","type":"function","function":{"name":"ApplyPatch","arguments":"{\\\"x\\\":1}"}}]}}]
        });
        let events = chat_chunk_to_responses_events(&mut st, &chunk);
        let names: Vec<String> = events.iter().map(|(e, _)| e.clone()).collect();
        assert!(names.contains(&"response.created".to_string()));
        assert!(names.contains(&"response.output_item.added".to_string()));
        assert!(names.contains(&"response.function_call_arguments.delta".to_string()));
    }
}
