pub mod agent;
pub mod channel;
pub mod compaction;
pub mod config;
pub mod cortex;
pub mod cron;
pub mod gateway;
pub mod llm;
pub mod mcp;
pub mod memory;
pub mod otel;
pub mod router;
pub mod session;
pub mod skill;
pub mod startup;
pub mod tokens;
pub mod tools;

/// Global mutex for tests that manipulate process-wide state (PATH, HOME, etc.).
/// Any test that sets or reads PATH must hold this lock for the duration of the test.
#[cfg(test)]
pub static TEST_PATH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());