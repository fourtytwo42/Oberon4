use oberon4_core::{OberonError, SecretStore};

pub fn store_provider_token<S: SecretStore>(
    store: &mut S,
    provider: &str,
    token: &str,
) -> Result<(), OberonError> {
    store.set_secret(&format!("provider.{provider}.token"), token)
}

#[cfg(test)]
mod tests {
    use super::store_provider_token;
    use oberon4_core::{InMemorySecretStore, SecretStore};

    #[test]
    fn auth_uses_secret_store_abstraction() {
        let mut store = InMemorySecretStore::default();
        store_provider_token(&mut store, "openai", "secret-token").expect("token should store");
        assert_eq!(
            store.get_secret("provider.openai.token").unwrap(),
            "secret-token"
        );
    }
}
