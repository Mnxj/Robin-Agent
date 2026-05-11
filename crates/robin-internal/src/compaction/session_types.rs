// Re-exports for compatibility. Compaction code uses crate::session directly.
pub use crate::session::{
    CompactionData, EntryType, ImageData, MessageData, Session, SessionEntry,
    ToolCallData, ToolResultData,
    assistant_message_entry, compaction_entry, tool_call_entry, tool_result_entry,
    user_message_entry,
};
