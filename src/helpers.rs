use chrono::Utc;
use once_cell::sync::Lazy;
use std::sync::{Arc, RwLock};

// ------ now_timestamp ------

static NOW_GETTER: Lazy<RwLock<Arc<dyn Fn() -> i64 + Send + Sync>>> =
    Lazy::new(|| RwLock::new(Arc::new(|| Utc::now().timestamp())));

pub fn set_now_getter(now: impl Fn() -> i64 + Send + Sync + 'static) {
    *NOW_GETTER.write().unwrap() = Arc::new(now);
}

/// Get the current timestamp.
///
/// It should be used instead of `chrono::Utc::now()` to allow time manipulation in tests.
///
/// There shouldn't be any noticeable performance penalties according to the benchmarks.
#[allow(clippy::must_use_candidate)]
pub fn now_timestamp() -> i64 {
    NOW_GETTER.read().unwrap()()
}
