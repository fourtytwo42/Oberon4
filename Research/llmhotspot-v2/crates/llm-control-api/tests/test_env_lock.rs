use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

fn env_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub struct EnvTestGuard {
    _lock: MutexGuard<'static, ()>,
    snapshot: HashMap<String, String>,
}

pub fn lock_env() -> EnvTestGuard {
    let lock = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
    let snapshot = std::env::vars().collect::<HashMap<_, _>>();
    std::env::set_var("LLMHOTSPOT_SECRET_STORE_APP_STATE_FALLBACK", "1");
    std::env::remove_var("LLMHOTSPOT_ALLOW_UNAUTH_NO_KEYS");
    EnvTestGuard {
        _lock: lock,
        snapshot,
    }
}

impl Drop for EnvTestGuard {
    fn drop(&mut self) {
        let current = std::env::vars().collect::<HashMap<_, _>>();
        for key in current.keys() {
            if !self.snapshot.contains_key(key) {
                std::env::remove_var(key);
            }
        }
        for (key, value) in &self.snapshot {
            std::env::set_var(key, value);
        }
    }
}
