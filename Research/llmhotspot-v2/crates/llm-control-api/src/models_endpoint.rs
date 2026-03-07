use super::*;
pub(crate) async fn advertised_model_ids(state: &AppState) -> Vec<String> {
    let mut model_ids: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    if let Some(store) = &state.store {
        if let Ok(accounts) = list_all_accounts(store).await {
            let has_openai = accounts.iter().any(|a| a.enabled && is_openai_account(a));
            let anthropic_accounts: Vec<_> = accounts
                .iter()
                .filter(|a| a.enabled && is_anthropic_account(a))
                .collect();
            let google_accounts: Vec<_> = accounts
                .iter()
                .filter(|a| a.enabled && is_google_account(a))
                .collect();
            let custom_accounts: Vec<_> = accounts
                .iter()
                .filter(|a| a.enabled && is_custom_openai_api_account(a))
                .collect();

            if has_openai {
                for id in state.gateway.models() {
                    if hidden_model_from_advertising(&id) {
                        continue;
                    }
                    if seen.insert(id.clone()) {
                        model_ids.push(id);
                    }
                }
            }

            if !anthropic_accounts.is_empty() {
                // Try the live model list cached by anthropic_oauth_models.
                // Fall back to known models when the cache is cold.
                let mut found_cached = false;
                for acct in &anthropic_accounts {
                    let key = cached_models_key("anthropic", &acct.account_id);
                    if let Some(env) = load_models_cache(store, &key, now_unix()).await {
                        let ids = parse_model_ids_from_cache_data(&env.data);
                        if !ids.is_empty() {
                            for id in ids {
                                if hidden_model_from_advertising(&id) {
                                    continue;
                                }
                                if seen.insert(id.clone()) {
                                    model_ids.push(id);
                                }
                            }
                            found_cached = true;
                            break;
                        }
                    }
                }
                if !found_cached {
                    for id in ANTHROPIC_KNOWN_MODELS.iter().map(|s| s.to_string()) {
                        if hidden_model_from_advertising(&id) {
                            continue;
                        }
                        if seen.insert(id.clone()) {
                            model_ids.push(id);
                        }
                    }
                }
            }

            if !google_accounts.is_empty() {
                let mut found_cached = false;
                for acct in &google_accounts {
                    let key = cached_models_key("google", &acct.account_id);
                    if let Some(env) = load_models_cache(store, &key, now_unix()).await {
                        let ids = parse_model_ids_from_cache_data(&env.data);
                        if !ids.is_empty() {
                            for id in ids {
                                if hidden_model_from_advertising(&id) {
                                    continue;
                                }
                                if seen.insert(id.clone()) {
                                    model_ids.push(id);
                                }
                            }
                            found_cached = true;
                            break;
                        }
                    }
                }
                if !found_cached {
                    for id in GOOGLE_KNOWN_MODELS.iter().map(|s| s.to_string()) {
                        if hidden_model_from_advertising(&id) {
                            continue;
                        }
                        if seen.insert(id.clone()) {
                            model_ids.push(id);
                        }
                    }
                }
            }

            if !custom_accounts.is_empty() {
                for acct in custom_accounts {
                    let ids = match refresh_custom_api_models_for_account(store, acct, false).await
                    {
                        Ok(v) => v,
                        Err(_) => {
                            let key = cached_models_key(OPENAI_API_PROVIDER_ID, &acct.account_id);
                            if let Some(env) = load_models_cache(store, &key, now_unix()).await {
                                parse_model_ids_from_cache_data(&env.data)
                            } else {
                                Vec::new()
                            }
                        }
                    };
                    for id in ids {
                        if hidden_model_from_advertising(&id) {
                            continue;
                        }
                        if seen.insert(id.clone()) {
                            model_ids.push(id);
                        }
                    }
                }
            }
        }
    } else {
        model_ids = state
            .gateway
            .models()
            .into_iter()
            .filter(|id| !hidden_model_from_advertising(id))
            .collect();
    }

    model_ids
}

pub(crate) async fn models(State(state): State<AppState>) -> impl IntoResponse {
    let model_ids = advertised_model_ids(&state).await;
    let model_objects: Vec<Value> = model_ids
        .into_iter()
        .map(|id| {
            let (context_window, max_tokens) = model_limits(&id);
            json!({
                "id": id,
                "object": "model",
                "created": 0,
                "owned_by": "system",
                "contextWindow": context_window,
                "context_window": context_window,
                "maxTokens": max_tokens,
                "max_tokens": max_tokens
            })
        })
        .collect();
    Json(json!({"object": "list", "data": model_objects}))
}

pub(crate) fn model_limits(model_id: &str) -> (u32, u32) {
    let id = model_id.to_ascii_lowercase();
    if id.contains("codex") {
        return (258_400, 32_768);
    }
    if id.starts_with("gpt-5") {
        return (128_000, 16_384);
    }
    if id.starts_with("claude") {
        // claude-opus-4 and claude-sonnet-4 families have 1M context (per openclaw live-model-filter)
        if id.contains("opus-4") {
            return (1_048_576, 128_000);
        }
        if id.contains("sonnet-4") {
            return (1_048_576, 64_000);
        }
        if id.contains("opus") {
            return (200_000, 32_768);
        }
        if id.contains("sonnet") {
            return (200_000, 64_000);
        }
        if id.contains("haiku") {
            return (200_000, 8_192);
        }
        return (200_000, 32_768);
    }
    (16_384, 4_096)
}
