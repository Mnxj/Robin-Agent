pub mod builder;
pub mod compaction;
pub mod overflow;
pub mod prompt;
pub mod session_types;
pub mod splitter;
pub mod summarizer;

pub use builder::{build_manager, Provider};
pub use compaction::{CompactionResult, Manager, Reason, MAX_CONSECUTIVE_FAILURES};
pub use overflow::is_context_overflow;
pub use prompt::{
    build_prompt, build_prompt_parts, build_transcript, format_compact_summary,
    transcript_delimiter_suffix,
};
pub use splitter::split;
pub use summarizer::{placeholder_summary, ErrEmptySummary, Summarizer};
