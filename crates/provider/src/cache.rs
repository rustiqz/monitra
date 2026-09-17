//! The embedded `Cache` default (DESIGN.md §4.1).
//!
//! Lives directly in `provider`, unlike the `Store`/`Notifier` defaults —
//! an in-process map needs no external protocol knowledge, so there is
//! nothing for a dedicated crate to own (contrast `storage`/`notify-webhook`,
//! which speak SQLite/HTTP respectively).

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::error::ProviderError;
use crate::traits::Cache;

#[derive(Default)]
pub struct InProcessCache {
    entries: Mutex<HashMap<String, String>>,
}

impl InProcessCache {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Cache for InProcessCache {
    fn name(&self) -> &'static str {
        "in-process"
    }

    async fn get(&self, key: &str) -> Result<Option<String>, ProviderError> {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        Ok(entries.get(key).cloned())
    }

    async fn set(&self, key: &str, value: &str) -> Result<(), ProviderError> {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        entries.insert(key.to_string(), value.to_string());
        Ok(())
    }
}
