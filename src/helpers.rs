use chrono::Utc;
use once_cell::sync::Lazy;
use std::sync::{Arc, RwLock};

// ------ now_timestamp ------

static NOW_GETTER: Lazy<RwLock<Arc<dyn Fn() -> i64 + Send + Sync>>> = Lazy::new(|| RwLock::new(Arc::new(|| Utc::now().timestamp())));

pub fn set_now_getter(now: impl Fn() -> i64 + Send + Sync + 'static) {
    *NOW_GETTER.write().unwrap() = Arc::new(now);
}

pub fn now_timestamp() -> i64 {
    NOW_GETTER.read().unwrap()()
}
