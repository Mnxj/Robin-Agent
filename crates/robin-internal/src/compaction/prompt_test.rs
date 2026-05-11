use crate::session::{
    compaction_entry, tool_call_entry, tool_result_entry,
    assistant_message_entry, user_message_entry,
};

use super::{
    build_prompt, build_transcript, format_compact_summary, transcript_delimiter_suffix,
};

#[test]
fn test_build_transcript_includes_all_roles() {
    let entries = vec![
        user_message_entry("how do I read a file?"),
        assistant_message_entry("use the read_file tool"),
        tool_call_entry("tc-1", "read_file", br#"{"path":"/tmp/x"}"#),
        tool_result_entry("tc-1", "file contents here", "", vec![]),
    ];
    let got = build_transcript(&entries);
    assert!(got.contains("USER: how do I read a file?"), "missing USER line");
    assert!(got.contains("ASSISTANT: use the read_file tool"), "missing ASSISTANT line");
    assert!(got.contains("TOOL_CALL[read_file]:"), "missing TOOL_CALL line");
    assert!(
        regex::Regex::new(r"TOOL_RESULT_[a-f0-9]+ \(untrusted, begin\):").unwrap().is_match(&got),
        "missing TOOL_RESULT begin marker"
    );
    assert!(got.contains("file contents here"), "missing tool result content");
    assert!(
        regex::Regex::new(r"TOOL_RESULT_[a-f0-9]+ \(end\)").unwrap().is_match(&got),
        "missing TOOL_RESULT end marker"
    );
}

#[test]
fn test_build_transcript_marks_errored_tool_result() {
    let entries = vec![
        tool_call_entry("tc-1", "bash", br#"{"cmd":"false"}"#),
        tool_result_entry("tc-1", "", "exit status 1", vec![]),
    ];
    let got = build_transcript(&entries);
    assert!(
        regex::Regex::new(r"TOOL_RESULT\[error\]_[a-f0-9]+ \(untrusted, begin\):").unwrap().is_match(&got),
        "missing error begin marker"
    );
    assert!(got.contains("exit status 1"), "missing error content");
    assert!(
        regex::Regex::new(r"TOOL_RESULT\[error\]_[a-f0-9]+ \(end\)").unwrap().is_match(&got),
        "missing error end marker"
    );
}

#[test]
fn test_build_prompt_no_extra_instructions() {
    let transcript = "USER: hi";
    let got = build_prompt(transcript, "");
    assert!(got.contains("summarizing"), "missing 'summarizing'");
    assert!(got.contains("USER: hi"), "missing transcript");
    assert!(!got.contains("Additional focus"), "unexpected 'Additional focus'");
}

#[test]
fn test_build_prompt_with_focus_instructions() {
    let got = build_prompt("USER: hi", "focus on API decisions");
    assert!(
        got.contains("Additional focus: focus on API decisions"),
        "missing additional focus"
    );
}

#[test]
fn test_build_transcript_folds_previous_summary() {
    let entries = vec![
        compaction_entry("earlier work: built X, decided Y", "", "", "m", 0, 0, 1),
        user_message_entry("now what about Z?"),
    ];
    let got = build_transcript(&entries);
    assert!(
        got.contains("PREVIOUS_SUMMARY: earlier work: built X, decided Y"),
        "missing PREVIOUS_SUMMARY"
    );
    assert!(got.contains("USER: now what about Z?"), "missing user message");
}

#[test]
fn test_prompt_includes_nine_sections() {
    let got = build_prompt("CONVERSATION HERE", "");
    for section in &[
        "1. Primary Request and Intent",
        "2. Key Technical Concepts",
        "3. Files and Code Sections",
        "4. Errors and fixes",
        "5. Problem Solving",
        "6. All user messages",
        "7. Pending Tasks",
        "8. Current Work",
        "9. Optional Next Step",
    ] {
        assert!(got.contains(section), "prompt missing section: {}", section);
    }
}

#[test]
fn test_prompt_demands_analysis_scratchpad() {
    let got = build_prompt("CONVERSATION HERE", "");
    assert!(got.contains("<analysis>"), "prompt must contain <analysis>");
    assert!(got.contains("<summary>"), "prompt must contain <summary>");
}

#[test]
fn test_prompt_requires_identifier_preservation() {
    let got = build_prompt("CONVERSATION HERE", "").to_lowercase();
    assert!(got.contains("verbatim"), "prompt must require verbatim preservation");
    for kind in &["file path", "uuid", "identifier"] {
        assert!(got.contains(kind), "prompt must mention {:?} identifiers", kind);
    }
}

#[test]
fn test_prompt_requires_all_user_messages_enumerated() {
    let got = build_prompt("CONVERSATION HERE", "").to_lowercase();
    assert!(got.contains("all user messages"), "prompt must require all user messages");
}

#[test]
fn test_prompt_requires_verbatim_next_step() {
    let got = build_prompt("CONVERSATION HERE", "").to_lowercase();
    assert!(got.contains("next step"), "prompt must include Optional Next Step");
    assert!(got.contains("verbatim"), "prompt must require verbatim quotes");
}

#[test]
fn test_prompt_includes_transcript() {
    let got = build_prompt("CONVERSATION GOES HERE", "");
    assert!(
        got.contains("CONVERSATION GOES HERE"),
        "transcript must be embedded in the prompt"
    );
}

#[test]
fn test_prompt_appends_additional_instructions() {
    let got = build_prompt("X", "focus on test failures");
    assert!(
        got.contains("focus on test failures"),
        "additional instructions must appear in the prompt"
    );
}

#[test]
fn test_format_compact_summary_strips_analysis() {
    let raw = "<analysis>\nchain of thought drafting\n</analysis>\n\n<summary>\n1. Primary Request: Build the thing.\n2. Key Tech: Go.\n</summary>";
    let got = format_compact_summary(raw);
    assert!(!got.contains("<analysis>"), "analysis tags must be stripped");
    assert!(
        !got.contains("chain of thought drafting"),
        "analysis content must be removed"
    );
    assert!(!got.contains("<summary>"), "summary tags must be replaced");
    assert!(got.contains("Summary:"), "summary must be wrapped under Summary: header");
    assert!(got.contains("Primary Request: Build the thing."));
}

#[test]
fn test_format_compact_summary_handles_missing_tags() {
    let raw = "User asked about X; we did Y.";
    let got = format_compact_summary(raw);
    assert!(got.contains("User asked about X"));
}

#[test]
fn test_format_compact_summary_handles_multiple_summary_blocks() {
    let raw = "<summary>first</summary>\n\n<summary>second</summary>";
    let got = format_compact_summary(raw);
    assert!(!got.contains("<summary>"), "no <summary> tags should remain");
    assert!(got.contains("first"));
    assert!(got.contains("second"));
}

#[test]
fn test_build_transcript_caps_large_tool_results() {
    let huge = "a".repeat(20_000);
    let entries = vec![tool_result_entry("tc1", &huge, "", vec![])];
    let got = build_transcript(&entries);
    assert!(
        got.len() < 12_000,
        "transcript must cap oversized tool results (got {} bytes)",
        got.len()
    );
    assert!(
        got.contains("[truncated"),
        "truncation marker must be present"
    );
}

#[test]
fn test_build_transcript_leaves_small_tool_results_intact() {
    let small = "small output line";
    let entries = vec![tool_result_entry("tc1", small, "", vec![])];
    let got = build_transcript(&entries);
    assert!(got.contains(small), "small tool results must be preserved verbatim");
}

#[test]
fn test_build_transcript_uses_random_delimiter_suffix() {
    let entries = vec![tool_result_entry("tc1", "hello", "", vec![])];
    let got1 = build_transcript(&entries);
    let got2 = build_transcript(&entries);

    let re = regex::Regex::new(r"TOOL_RESULT_([a-f0-9]+) \(untrusted, begin\)").unwrap();
    let m1 = re.captures(&got1).expect("first transcript must have TOOL_RESULT marker");
    let m2 = re.captures(&got2).expect("second transcript must have TOOL_RESULT marker");

    let s1 = m1.get(1).unwrap().as_str();
    let s2 = m2.get(1).unwrap().as_str();
    assert_ne!(s1, s2, "per-call suffix must differ across BuildTranscript invocations");
    assert!(s1.len() >= 8, "suffix must be at least 8 hex chars");
}

#[test]
fn test_build_transcript_suffix_is_uniform_within_one_transcript() {
    let entries = vec![
        tool_result_entry("tc1", "first", "", vec![]),
        tool_result_entry("tc2", "second", "", vec![]),
        tool_result_entry("tc3", "third", "an error", vec![]),
    ];
    let got = build_transcript(&entries);
    let re = regex::Regex::new(r"TOOL_RESULT(?:\[error\])?_([a-f0-9]+) \((?:untrusted, begin|end)\)").unwrap();
    let matches: Vec<_> = re.captures_iter(&got).collect();
    assert!(
        matches.len() >= 6,
        "expected at least 3 begin + 3 end markers, got {}",
        matches.len()
    );
    let first = matches[0].get(1).unwrap().as_str();
    for (i, m) in matches.iter().enumerate() {
        let s = m.get(1).unwrap().as_str();
        assert_eq!(
            s, first,
            "marker {} suffix {:?} must equal first suffix {:?}",
            i, s, first
        );
    }
}

#[test]
fn test_build_transcript_error_marker_still_uses_untrusted_wrapping() {
    let entries = vec![tool_result_entry("tc1", "ignored", "boom: file not found", vec![])];
    let got = build_transcript(&entries);
    assert!(got.contains("TOOL_RESULT[error]_"), "error label must be preserved");
    assert!(got.contains("(untrusted, begin):"), "error results must also use untrusted wrapping");
    assert!(got.contains("boom: file not found"), "error text must be present");
}

#[test]
fn test_transcript_delimiter_suffix_is_always_hex() {
    for i in 0..50 {
        let got = transcript_delimiter_suffix();
        assert_eq!(got.len(), 8, "suffix must be 8 hex chars (iteration {})", i);
        assert!(
            hex::decode(&got).is_ok(),
            "suffix must be valid hex (iteration {}, got {:?})",
            i,
            got
        );
    }
}