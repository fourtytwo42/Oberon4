#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod codex_contract_tests {
    use super::{
        build_codex_payload, build_codex_request_headers, is_protocol_leakage_line,
        project_chat_stream_from_responses_sse, sanitize_assistant_text,
    };
    use serde_json::Value;

    fn extract_text_from_codex_response(body: &Value) -> Option<String> {
        body.get("output")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|first| first.get("content"))
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .and_then(|c| c.get("text"))
            .and_then(Value::as_str)
            .map(str::to_string)
    }
    use serde_json::json;
    use std::sync::Mutex;

    static SAFE_TEXT_ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn tool_choice_required_on_first_turn_auto_on_followup() {
        let tool = json!([{"type":"function","function":{"name":"run_cmd","parameters":{"type":"object"}}}]);

        // First turn: no tool results â†’ must force "required"
        let first_turn = json!({
            "model": "gpt-5.2",
            "messages": [{"role":"user","content":"scaffold a node project"}],
            "tools": tool,
        });
        let payload = build_codex_payload(&first_turn, "ws_1");
        assert_eq!(
            payload.get("tool_choice").and_then(|v| v.as_str()),
            Some("required"),
            "first turn should be required"
        );

        // Follow-up turn: tool result present â†’ must allow "auto" so model can finish
        let followup = json!({
            "model": "gpt-5.2",
            "messages": [
                {"role":"user","content":"scaffold a node project"},
                {"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"run_cmd","arguments":"{}"}}]},
                {"role":"tool","tool_call_id":"call_1","content":"ok"},
            ],
            "tools": tool,
        });
        let payload2 = build_codex_payload(&followup, "ws_1");
        assert_eq!(
            payload2.get("tool_choice").and_then(|v| v.as_str()),
            Some("auto"),
            "follow-up turn should be auto so model can give final answer"
        );
    }

    #[test]
    fn codex_payload_transforms_chat_messages() {
        let input = json!({
            "model":"gpt-5.2",
            "messages":[
                {"role":"system","content":"sys inst"},
                {"role":"user","content":"hello"}
            ],
            "tools":[{"type":"function","function":{"name":"t1","parameters":{"type":"object"}}}],
            "tool_choice":"required",
            "stream":true
        });
        let payload = build_codex_payload(&input, "ws_1");
        assert_eq!(
            payload.get("model").and_then(|v| v.as_str()),
            Some("gpt-5.2")
        );
        assert_eq!(payload.get("stream").and_then(|v| v.as_bool()), Some(true));
        assert!(payload.get("input").and_then(|v| v.as_array()).is_some());
        assert!(payload.get("tools").and_then(|v| v.as_array()).is_some());
        assert_eq!(
            payload.get("prompt_cache_key").and_then(|v| v.as_str()),
            Some("ws_1")
        );
    }

    #[test]
    fn codex_payload_normalizes_overlong_call_id_for_custom_tool_items() {
        // Cursor Agent sometimes uses `custom_tool_call` with long IDs that Codex rejects (>64).
        let long_id = "call_custom_ctc_06d7341fbbeb5dd0016990c981d66481a182fdbea6635f1377"; // 66 chars
        assert!(long_id.len() > 64);

        let input = json!({
            "model": "gpt-5.2",
            "input": [
                {"type":"custom_tool_call","call_id": long_id, "name":"ApplyPatch", "input":"{}"},
                {"type":"custom_tool_call_output","call_id": long_id, "output":"ok"}
            ],
            "stream": true
        });
        let payload = build_codex_payload(&input, "ws_1");
        let arr = payload
            .get("input")
            .and_then(|v| v.as_array())
            .expect("input array");
        let mapped_0 = arr[0].get("call_id").and_then(|v| v.as_str()).unwrap();
        let mapped_1 = arr[1].get("call_id").and_then(|v| v.as_str()).unwrap();
        assert_eq!(mapped_0, mapped_1, "call_id remap must be consistent");
        assert_eq!(mapped_0.len(), 64);
        assert!(mapped_0.starts_with("cid_"));
    }

    #[test]
    fn codex_headers_include_required_fields() {
        let jwt = "eyJhbGciOiJub25lIn0.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjdF8xIn19.";
        let headers = build_codex_request_headers(jwt, "ws_1", "https://api.openai.com/auth");
        assert_eq!(
            headers.get("openai-beta").map(String::as_str),
            Some("responses=experimental")
        );
        assert_eq!(headers.get("originator").map(String::as_str), Some("pi"));
        assert_eq!(headers.get("session_id").map(String::as_str), Some("ws_1"));
        assert_eq!(
            headers.get("chatgpt-account-id").map(String::as_str),
            Some("acct_1")
        );
    }

    #[test]
    fn codex_response_text_extraction_works() {
        let body = json!({
            "output":[{"content":[{"text":"hello world"}]}]
        });
        assert_eq!(
            extract_text_from_codex_response(&body).as_deref(),
            Some("hello world")
        );
    }

    #[test]
    fn responses_sse_projection_preserves_tool_calls() {
        let raw = [
            "data: {\"type\":\"response.created\",\"response\":{\"model\":\"gpt-5.1-codex-mini\"}}",
            "",
            "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"exec_command\",\"arguments\":\"\"}}",
            "",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_1\",\"delta\":\"{\\\"cmd\\\":\\\"ls\"}",
            "",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_1\",\"delta\":\" -la\\\"}\"}",
            "",
            "data: {\"type\":\"response.function_call_arguments.done\",\"item_id\":\"fc_1\",\"arguments\":\"{\\\"cmd\\\":\\\"ls -la\\\"}\"}",
            "",
            "data: {\"type\":\"response.completed\",\"response\":{\"model\":\"gpt-5.1-codex-mini\"}}",
            "",
            "data: [DONE]",
            "",
        ]
        .join("\n");
        let projected = project_chat_stream_from_responses_sse(&raw);
        assert_eq!(projected.model.as_deref(), Some("gpt-5.1-codex-mini"));
        assert_eq!(projected.tool_calls.len(), 1);
        assert_eq!(
            projected.tool_calls[0]
                .get("function")
                .and_then(|v| v.get("name"))
                .and_then(|v| v.as_str()),
            Some("exec_command")
        );
        assert_eq!(
            projected.tool_calls[0]
                .get("function")
                .and_then(|v| v.get("arguments"))
                .and_then(|v| v.as_str()),
            Some("{\"cmd\":\"ls -la\"}")
        );
    }

    #[test]
    fn responses_sse_projection_reads_message_output_items() {
        let raw = [
            "data: {\"type\":\"response.created\",\"response\":{\"model\":\"gpt-5.1-codex-mini\"}}",
            "",
            "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"hello from item\"}]}}",
            "",
            "data: [DONE]",
            "",
        ]
        .join("\n");
        let projected = project_chat_stream_from_responses_sse(&raw);
        assert_eq!(projected.text, "hello from item");
    }

    #[test]
    fn responses_sse_projection_extracts_token_counts() {
        let raw = [
            "data: {\"type\":\"response.created\",\"response\":{\"model\":\"gpt-5.2\"}}",
            "",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello!\"}",
            "",
            "data: {\"type\":\"response.completed\",\"response\":{\"model\":\"gpt-5.2\",\"usage\":{\"input_tokens\":42,\"output_tokens\":13,\"total_tokens\":55}}}",
            "",
            "data: [DONE]",
            "",
        ]
        .join("\n");
        let projected = project_chat_stream_from_responses_sse(&raw);
        assert_eq!(projected.text, "Hello!");
        assert_eq!(projected.usage_prompt_tokens, 42);
        assert_eq!(projected.usage_completion_tokens, 13);
        assert_eq!(projected.usage_total_tokens, 55);
    }

    #[test]
    fn protocol_leakage_line_detects_patch_envelopes() {
        assert!(is_protocol_leakage_line("*** Begin Patch"));
        assert!(is_protocol_leakage_line("*** End Patch"));
        assert!(is_protocol_leakage_line("*** Update File: src/lib.rs"));
        assert!(is_protocol_leakage_line("*** Add File: src/new.rs"));
        assert!(is_protocol_leakage_line("*** Delete File: src/old.rs"));
        assert!(is_protocol_leakage_line("*** Move to: src/new_name.rs"));
    }

    #[test]
    fn protocol_leakage_line_detects_tool_payload_markers() {
        assert!(is_protocol_leakage_line(
            "{\"recipient_name\":\"functions.exec_command\",\"parameters\":{\"cmd\":\"pwd\"}}"
        ));
        assert!(is_protocol_leakage_line(
            "{\"recipient_name\": \"functions.exec_command\", \"parameters\": {\"cmd\":\"pwd\"}}"
        ));
        assert!(is_protocol_leakage_line(
            "{\"recipient_name\":\"multi_tool_use.parallel\",\"parameters\":{\"tool_uses\":[]}}"
        ));
        assert!(is_protocol_leakage_line(
            "{\"tool_uses\":[{\"recipient_name\":\"functions.exec_command\",\"parameters\":{\"cmd\":\"ls\"}}]}"
        ));
        assert!(is_protocol_leakage_line(
            "{\"tool_uses\": [{\"recipient_name\": \"functions.exec_command\", \"parameters\": {\"cmd\": \"ls\"}}]}"
        ));
        assert!(is_protocol_leakage_line("to=functions.exec_command"));
        assert!(is_protocol_leakage_line("to=multi_tool_use.parallel"));
        // Bash/shell tool parameter blobs
        assert!(is_protocol_leakage_line(
            "{\"command\":\"ls -la\",\"description\":\"List files\",\"working_directory\":\"/tmp\"}"
        ));
        assert!(is_protocol_leakage_line(
            "{\"command\":\"python run.py\",\"description\":\"Run script\",\"block_until_ms\":120000}"
        ));
        assert!(is_protocol_leakage_line(
            "{\"command\":\"cargo build\",\"timeout\":60000}"
        ));
        // Edit tool parameter blobs
        assert!(is_protocol_leakage_line(
            "{\"file_path\":\"src/lib.rs\",\"old_string\":\"foo\",\"new_string\":\"bar\"}"
        ));
        // Agent dispatch blobs
        assert!(is_protocol_leakage_line(
            "{\"subagent_type\":\"general-purpose\",\"prompt\":\"do something\"}"
        ));
        // Should NOT filter short/trivial command JSON (less than 9 chars)
        assert!(!is_protocol_leakage_line("{\"command\":\"ls\"}"));
    }

    #[test]
    fn protocol_leakage_line_detects_multiline_json_fragments() {
        assert!(is_protocol_leakage_line(
            r#""recipient_name": "functions.exec_command","#,
        ));
        assert!(is_protocol_leakage_line(
            r#""recipient_name": "multi_tool_use.parallel","#,
        ));
    }

    #[test]
    fn detect_text_mode_tool_call_bridges_known_blobs() {
        use super::detect_text_mode_tool_call;
        // Bash blob → ("Bash", args)
        let bash = r#"{"command":"python run.py","description":"Run training","working_directory":"/tmp"}"#;
        let r = detect_text_mode_tool_call(bash).expect("should detect Bash blob");
        assert_eq!(r.0, "Bash");
        assert_eq!(r.1, bash);
        // Bash with block_until_ms
        let bash2 = r#"{"command":"cargo build","description":"Build","block_until_ms":60000}"#;
        assert_eq!(
            detect_text_mode_tool_call(bash2).map(|r| r.0).as_deref(),
            Some("Bash")
        );
        // Edit blob → ("Edit", args)
        let edit = r#"{"file_path":"src/lib.rs","old_string":"foo","new_string":"bar"}"#;
        assert_eq!(
            detect_text_mode_tool_call(edit).map(|r| r.0).as_deref(),
            Some("Edit")
        );
        // Agent blob → ("Agent", args)
        let agent = r#"{"subagent_type":"general-purpose","prompt":"do something"}"#;
        assert_eq!(
            detect_text_mode_tool_call(agent).map(|r| r.0).as_deref(),
            Some("Agent")
        );
        // Plain command-only JSON (no secondary field) → None (avoid false positives)
        assert!(detect_text_mode_tool_call(r#"{"command":"ls"}"#).is_none());
        // Random JSON → None
        assert!(detect_text_mode_tool_call(r#"{"foo":"bar"}"#).is_none());
        // Not JSON → None
        assert!(detect_text_mode_tool_call("just some text").is_none());
    }

    #[test]
    fn sanitize_assistant_text_filters_protocol_lines_only() {
        let raw = [
            "Normal explanation line",
            "*** Begin Patch",
            "{\"recipient_name\":\"functions.exec_command\",\"parameters\":{\"cmd\":\"pwd\"}}",
            "Another normal line",
        ]
        .join("\n");
        let out = sanitize_assistant_text(&raw);
        assert_eq!(out, "Normal explanation line\nAnother normal line");
    }

    #[test]
    fn sanitize_assistant_text_preserves_false_positive_prose() {
        let raw =
            "Explain patch strategy and tool arguments. Mention recipient_name as a field in docs.";
        assert_eq!(sanitize_assistant_text(raw), raw);
        let raw = "The keys \"tool_uses\", \"recipient_name\", and \"parameters\" are examples in this sentence.";
        assert_eq!(sanitize_assistant_text(raw), raw);
    }

    #[test]
    fn sanitize_assistant_text_respects_safe_mode_off() {
        let _g = SAFE_TEXT_ENV_LOCK.lock().expect("env lock");
        std::env::set_var("LLMHOTSPOT_SAFE_TEXT_MODE", "off");
        let raw = "*** Begin Patch\nKeep this in rollback mode";
        let out = sanitize_assistant_text(raw);
        std::env::remove_var("LLMHOTSPOT_SAFE_TEXT_MODE");
        assert_eq!(out, raw);
    }

    #[test]
    fn responses_sse_projection_sanitizes_text_but_keeps_tool_calls_exact() {
        let raw = [
            "data: {\"type\":\"response.created\",\"response\":{\"model\":\"gpt-5.2\"}}",
            "",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Visible intro.\\n\"}",
            "",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"*** Begin Patch\\n\"}",
            "",
            "data: {\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"exec_command\",\"arguments\":\"\"}}",
            "",
            "data: {\"type\":\"response.function_call_arguments.done\",\"item_id\":\"fc_1\",\"arguments\":\"{\\\"cmd\\\":\\\"ls -la\\\"}\"}",
            "",
            "data: [DONE]",
            "",
        ]
        .join("\n");
        let projected = project_chat_stream_from_responses_sse(&raw);
        assert_eq!(projected.text, "Visible intro.\n");
        assert_eq!(projected.tool_calls.len(), 1);
        assert_eq!(
            projected.tool_calls[0].get("id").and_then(|v| v.as_str()),
            Some("call_1")
        );
        assert_eq!(
            projected.tool_calls[0]
                .get("function")
                .and_then(|v| v.get("name"))
                .and_then(|v| v.as_str()),
            Some("exec_command")
        );
        assert_eq!(
            projected.tool_calls[0]
                .get("function")
                .and_then(|v| v.get("arguments"))
                .and_then(|v| v.as_str()),
            Some("{\"cmd\":\"ls -la\"}")
        );
    }

    #[test]
    fn responses_sse_projection_bridges_text_mode_tool_blob_into_tool_calls() {
        let raw = [
            "data: {\"type\":\"response.created\",\"response\":{\"model\":\"gpt-5.2\"}}",
            "",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"{\\\"command\\\":\\\"python train.py\\\",\\\"description\\\":\\\"Run\\\",\\\"working_directory\\\":\\\"/repo\\\"}\\n\"}",
            "",
            "data: [DONE]",
            "",
        ]
        .join("\n");
        let projected = project_chat_stream_from_responses_sse(&raw);
        assert_eq!(projected.text, "");
        assert_eq!(projected.tool_calls.len(), 1);
        assert_eq!(
            projected.tool_calls[0]
                .get("function")
                .and_then(|v| v.get("name"))
                .and_then(|v| v.as_str()),
            Some("Bash")
        );
        assert_eq!(
            projected.tool_calls[0]
                .get("function")
                .and_then(|v| v.get("arguments"))
                .and_then(|v| v.as_str()),
            Some("{\"command\":\"python train.py\",\"description\":\"Run\",\"working_directory\":\"/repo\"}")
        );
    }

    #[test]
    fn codex_payload_preserves_tool_context_messages() {
        let input = json!({
            "model":"gpt-5.2",
            "messages":[
                {"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"exec_command","arguments":"{\"cmd\":\"pwd\"}"}}]},
                {"role":"tool","tool_call_id":"call_1","content":"{\"stdout\":\"/repo\"}"}
            ]
        });
        let payload = build_codex_payload(&input, "ws_1");
        let items = payload
            .get("input")
            .and_then(|v| v.as_array())
            .expect("input array");
        assert_eq!(
            items[0].get("type").and_then(|v| v.as_str()),
            Some("function_call")
        );
        assert_eq!(
            items[1].get("type").and_then(|v| v.as_str()),
            Some("function_call_output")
        );
    }

    #[test]
    fn codex_payload_drops_orphan_tool_output_without_previous_response_id() {
        let input = json!({
            "model":"gpt-5.2",
            "input":[
                {"type":"function_call_output","call_id":"call_missing","output":"ok"}
            ]
        });
        let payload = build_codex_payload(&input, "ws_1");
        let items = payload
            .get("input")
            .and_then(|v| v.as_array())
            .expect("input array");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].get("role").and_then(|v| v.as_str()), Some("user"));
        assert_eq!(
            items[0]
                .get("content")
                .and_then(|v| v.as_array())
                .and_then(|parts| parts.first())
                .and_then(|part| part.get("type"))
                .and_then(|v| v.as_str()),
            Some("input_text")
        );
    }

    #[test]
    fn codex_payload_keeps_tool_output_with_previous_response_id() {
        let input = json!({
            "model":"gpt-5.2",
            "previous_response_id":"resp_prev_1",
            "input":[
                {"type":"function_call_output","call_id":"call_missing","output":"ok"}
            ]
        });
        let payload = build_codex_payload(&input, "ws_1");
        assert_eq!(
            payload.get("previous_response_id").and_then(|v| v.as_str()),
            Some("resp_prev_1")
        );
        let items = payload
            .get("input")
            .and_then(|v| v.as_array())
            .expect("input array");
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].get("type").and_then(|v| v.as_str()),
            Some("function_call_output")
        );
    }
}

const CONSOLE_HTML: &str = include_str!("../assets/console.html");

#[cfg(test)]
mod tool_mismatch_repair_tests {
    use super::{normalize_openai_tools_inplace, normalize_tool_name_for_declaration};
    use serde_json::json;

    #[test]
    fn tool_name_normalization_handles_common_case_mismatches() {
        assert_eq!(
            normalize_tool_name_for_declaration("functions.ApplyPatch"),
            "apply_patch"
        );
        assert_eq!(
            normalize_tool_name_for_declaration("multi_tool_use.parallel"),
            "parallel"
        );
        assert_eq!(
            normalize_tool_name_for_declaration("functions.exec_command"),
            "exec_command"
        );
    }

    #[test]
    fn tools_recipient_name_mismatch_is_repaired() {
        let mut req = json!({
            "model":"gpt-5.2",
            "messages":[{"role":"user","content":"hi"}],
            "tools":[{"recipient_name":"functions.ApplyPatch","parameters":{"path":"x.txt","content":"ok"}}]
        });
        let out = normalize_openai_tools_inplace(&mut req);
        assert!(out.repaired);
        assert!(!out.unrecoverable);
        let tools = req
            .get("tools")
            .and_then(|v| v.as_array())
            .expect("tools array");
        assert_eq!(tools.len(), 1);
        assert_eq!(
            tools[0].get("type").and_then(|v| v.as_str()),
            Some("function")
        );
        assert_eq!(
            tools[0]
                .get("function")
                .and_then(|v| v.get("name"))
                .and_then(|v| v.as_str()),
            Some("apply_patch")
        );
    }

    #[test]
    fn tools_tool_uses_mismatch_is_repaired() {
        let mut req = json!({
            "model":"gpt-5.2",
            "messages":[{"role":"user","content":"hi"}],
            "tools":[{"tool_uses":[
                {"recipient_name":"functions.exec_command","parameters":{"cmd":"pwd"}},
                {"recipient_name":"functions.ApplyPatch","parameters":{"content":"ok"}}
            ]}]
        });
        let out = normalize_openai_tools_inplace(&mut req);
        assert!(out.repaired);
        assert!(!out.unrecoverable);
        let tools = req
            .get("tools")
            .and_then(|v| v.as_array())
            .expect("tools array");
        assert_eq!(tools.len(), 2);
        assert_eq!(
            tools[0]
                .get("function")
                .and_then(|v| v.get("name"))
                .and_then(|v| v.as_str()),
            Some("exec_command")
        );
        assert_eq!(
            tools[1]
                .get("function")
                .and_then(|v| v.get("name"))
                .and_then(|v| v.as_str()),
            Some("apply_patch")
        );
    }

    #[test]
    fn tools_non_array_is_unrecoverable() {
        let mut req = json!({
            "model":"gpt-5.2",
            "messages":[{"role":"user","content":"hi"}],
            "tools":{"recipient_name":"functions.exec_command"}
        });
        let out = normalize_openai_tools_inplace(&mut req);
        assert!(out.unrecoverable);
    }
}

#[cfg(test)]
mod anthropic_payload_tests {
    use super::{
        anthropic_messages_to_openai_chat, build_anthropic_payload, resolve_anthropic_model,
    };
    use serde_json::json;

    #[test]
    fn anthropic_required_downgrades_to_auto_after_tool_result() {
        let req = json!({
            "model": "claude-4.6-sonnet-medium",
            "messages": [
                {"role":"user","content":"Write a file."},
                {"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"write_file","arguments":"{\"path\":\"story.txt\",\"content\":\"one\"}"}}]},
                {"role":"tool","tool_call_id":"call_1","content":"ok"},
                {"role":"user","content":"Now summarize what you wrote in one sentence."}
            ],
            "tools": [{"type":"function","function":{"name":"write_file","parameters":{"type":"object"}}}],
            "tool_choice": "required"
        });
        let payload = build_anthropic_payload(&req);
        assert_eq!(
            payload
                .get("tool_choice")
                .and_then(|v| v.get("type"))
                .and_then(|v| v.as_str()),
            Some("auto")
        );
    }

    #[test]
    fn anthropic_none_omits_tools() {
        let req = json!({
            "model": "claude-4.6-sonnet-medium",
            "messages": [{"role":"user","content":"Say hello."}],
            "tools": [{"type":"function","function":{"name":"write_file","parameters":{"type":"object"}}}],
            "tool_choice": "none"
        });
        let payload = build_anthropic_payload(&req);
        assert!(payload.get("tools").is_none());
        assert!(payload.get("tool_choice").is_none());
    }

    #[test]
    fn anthropic_messages_any_becomes_none_on_pure_tool_result_followup() {
        let req = json!({
            "model": "claude-sonnet-4-6",
            "messages": [
                {"role":"user","content":[{"type":"text","text":"Write file"}]},
                {"role":"assistant","content":[{"type":"tool_use","id":"call_1","name":"write_file","input":{"path":"story.txt","content":"one"}}]},
                {"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":"ok"}]}
            ],
            "tools": [{"name":"write_file","input_schema":{"type":"object","properties":{"path":{"type":"string"}}}}],
            "tool_choice": {"type":"any"}
        });
        let out = anthropic_messages_to_openai_chat(&req).expect("convert");
        assert_eq!(
            out.get("tool_choice").and_then(|v| v.as_str()),
            Some("none")
        );
        assert!(out.get("tools").is_none());
    }

    #[test]
    fn anthropic_messages_none_omits_tools() {
        let req = json!({
            "model": "claude-sonnet-4-6",
            "messages": [{"role":"user","content":[{"type":"text","text":"Say hi"}]}],
            "tools": [{"name":"write_file","input_schema":{"type":"object","properties":{"path":{"type":"string"}}}}],
            "tool_choice": {"type":"none"}
        });
        let out = anthropic_messages_to_openai_chat(&req).expect("convert");
        assert!(out.get("tools").is_none());
        assert_eq!(
            out.get("tool_choice").and_then(|v| v.as_str()),
            Some("none")
        );
    }

    #[test]
    fn openai_tool_followup_omits_tools_for_anthropic_payload() {
        let req = json!({
            "model": "claude-sonnet-4-6",
            "messages": [
                {"role":"user","content":"Write a short story file."},
                {"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"write_file","arguments":"{\"path\":\"cat.txt\",\"content\":\"x\"}"}}]},
                {"role":"tool","tool_call_id":"call_1","content":"ok"}
            ],
            "tools": [{"type":"function","function":{"name":"write_file","parameters":{"type":"object"}}}],
            "tool_choice":"required"
        });
        let payload = build_anthropic_payload(&req);
        assert!(payload.get("tools").is_none());
        assert!(payload.get("tool_choice").is_none());
    }

    #[test]
    fn openai_user_tool_result_block_followup_omits_tools_for_anthropic_payload() {
        let req = json!({
            "model": "claude-sonnet-4-6",
            "messages": [
                {"role":"user","content":"Write a short story file."},
                {"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"write_file","arguments":"{\"path\":\"chapter1.md\",\"content\":\"once\"}"}}]},
                {"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":"ok"}]}
            ],
            "tools": [{"type":"function","function":{"name":"write_file","parameters":{"type":"object"}}}],
            "tool_choice":"auto"
        });
        let payload = build_anthropic_payload(&req);
        assert!(payload.get("tools").is_none());
        assert!(payload.get("tool_choice").is_none());
    }

    #[test]
    fn claude_aliases_normalize_to_canonical_models() {
        assert_eq!(
            resolve_anthropic_model("claude-4.6-opus-high-thinking"),
            "claude-opus-4-6"
        );
        assert_eq!(
            resolve_anthropic_model("claude-4.6-sonnet-medium"),
            "claude-sonnet-4-6"
        );
        assert_eq!(
            resolve_anthropic_model("claude-3.7-sonnet"),
            "claude-3-7-sonnet"
        );
    }

    #[test]
    fn anthropic_payload_uses_canonical_model_for_claude_alias() {
        let req = json!({
            "model": "claude-4.6-opus-high-thinking",
            "messages": [{"role":"user","content":"hello"}]
        });
        let payload = build_anthropic_payload(&req);
        assert_eq!(
            payload.get("model").and_then(|v| v.as_str()),
            Some("claude-opus-4-6")
        );
    }
}

#[cfg(test)]
mod tool_choice_normalization_tests {
    use super::{google_generate_to_openai_chat, normalize_openai_tool_choice_inplace};
    use serde_json::json;

    #[test]
    fn openai_tool_choice_string_any_normalizes_to_required() {
        let mut req = json!({
            "model":"gpt-5.3-codex",
            "messages":[{"role":"user","content":"hi"}],
            "tool_choice":"any"
        });
        normalize_openai_tool_choice_inplace(&mut req);
        assert_eq!(
            req.get("tool_choice").and_then(|v| v.as_str()),
            Some("required")
        );
    }

    #[test]
    fn openai_tool_choice_object_auto_normalizes_to_string_auto() {
        let mut req = json!({
            "model":"gpt-5.3-codex",
            "messages":[{"role":"user","content":"hi"}],
            "tool_choice":{"type":"auto"}
        });
        normalize_openai_tool_choice_inplace(&mut req);
        assert_eq!(
            req.get("tool_choice").and_then(|v| v.as_str()),
            Some("auto")
        );
    }

    #[test]
    fn google_any_tool_mode_maps_to_openai_required_string() {
        let req = json!({
            "contents":[{"role":"user","parts":[{"text":"hello"}]}],
            "toolConfig":{"functionCallingConfig":{"mode":"ANY"}}
        });
        let out = google_generate_to_openai_chat("gemini-2.5-flash", &req, false).expect("map");
        assert_eq!(
            out.get("tool_choice").and_then(|v| v.as_str()),
            Some("required")
        );
    }

    #[test]
    fn openai_tool_choice_json_string_auto_normalizes() {
        let mut req = json!({
            "model":"gpt-5.3-codex",
            "messages":[{"role":"user","content":"hi"}],
            "tool_choice":"{\"type\":\"auto\"}"
        });
        normalize_openai_tool_choice_inplace(&mut req);
        assert_eq!(
            req.get("tool_choice").and_then(|v| v.as_str()),
            Some("auto")
        );
    }

    #[test]
    fn openai_tool_choice_unknown_shape_is_removed() {
        let mut req = json!({
            "model":"gpt-5.3-codex",
            "messages":[{"role":"user","content":"hi"}],
            "tool_choice":{"type":"bogus"}
        });
        normalize_openai_tool_choice_inplace(&mut req);
        assert!(req.get("tool_choice").is_none());
    }
}

#[cfg(test)]
mod google_model_normalization_tests {
    use super::{
        resolve_google_model, resolve_provider_for_model, validate_chat_completions_strict,
    };
    use serde_json::json;

    #[test]
    fn maps_flash_preview_alias_to_flash() {
        assert_eq!(
            resolve_google_model("gemini-3-flash-preview"),
            "gemini-3-flash"
        );
        assert_eq!(
            resolve_google_model("gemini-3 flash preview"),
            "gemini-3-flash"
        );
    }

    #[test]
    fn strips_trailing_yyyymmdd_suffix() {
        assert_eq!(
            resolve_google_model("gemini-3-flash-20260228"),
            "gemini-3-flash"
        );
    }

    #[test]
    fn resolves_provider_from_prefix() {
        assert_eq!(resolve_provider_for_model("gpt-oss-120b"), Some("openai"));
        assert_eq!(
            resolve_provider_for_model("claude-opus-4-6"),
            Some("anthropic")
        );
        assert_eq!(
            resolve_provider_for_model("gemini-3.1-pro-preview"),
            Some("google")
        );
        assert!(resolve_provider_for_model("llama-3.3").is_none());
    }

    #[test]
    fn chat_validator_accepts_google_function_declarations_shape() {
        let req = json!({
            "model":"gemini-3-flash",
            "messages":[{"role":"user","content":"hi"}],
            "tools":[{
              "functionDeclarations":[
                {"name":"web_search","parameters":{"type":"object","properties":{}}}
              ]
            }]
        });
        assert!(validate_chat_completions_strict(&req).is_ok());
    }

    #[test]
    fn chat_validator_accepts_anthropic_tool_shape() {
        let req = json!({
            "model":"claude-sonnet-4-6",
            "messages":[{"role":"user","content":"hi"}],
            "tools":[{"name":"read_file","input_schema":{"type":"object","properties":{"path":{"type":"string"}}}}]
        });
        assert!(validate_chat_completions_strict(&req).is_ok());
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod console_ui_theme_tests {
    use super::{render_browser_callback_redirect_page, CONSOLE_HTML};

    #[test]
    fn console_html_includes_theme_toggle_and_theme_aware_brand_assets() {
        assert!(CONSOLE_HTML.contains("llmhotspot_theme"));
        assert!(CONSOLE_HTML.contains("prefers-color-scheme: dark"));
        assert!(CONSOLE_HTML.contains("id=\"favicon\""));
        assert!(CONSOLE_HTML.contains("/console-logo-dark.png"));
        assert!(CONSOLE_HTML.contains("/console-logo-light.png"));
        assert!(CONSOLE_HTML.contains("id=\"brandLogo\""));
        assert!(CONSOLE_HTML.contains("id=\"themeSeg\""));
        assert!(CONSOLE_HTML.contains("setThemeMode('auto')"));
        assert!(CONSOLE_HTML.contains("setThemeMode('light')"));
        assert!(CONSOLE_HTML.contains("setThemeMode('dark')"));
    }

    #[test]
    fn oauth_redirect_page_is_theme_aware_and_uses_local_logo_endpoints() {
        let html =
            render_browser_callback_redirect_page("ok", "sub", "http://localhost:11434/console");
        assert!(!html.contains("llmhotspot.com/logo.png"));
        assert!(html.contains("llmhotspot_theme"));
        assert!(html.contains("prefers-color-scheme"));
        assert!(html.contains("/console-logo-dark.png"));
        assert!(html.contains("/console-logo-light.png"));
    }
}

