#[cfg(test)]
mod tests {
    use crate::memory::memory::{Entry, Manager, extract_title, format_for_prompt};
    use std::path::PathBuf;

    fn make_entry(id: &str, content: &str) -> Entry {
        Entry {
            id: id.to_string(),
            title: extract_title(id, content),
            content: content.to_string(),
            file_path: PathBuf::new(),
            mod_time: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_memory_manager_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = Manager::new(dir.path());
        mgr.load().await.unwrap();

        // Save an entry.
        mgr.save("test-entry", "# Test Entry\n\nThis is a test memory about Go programming.")
            .await
            .unwrap();

        // Check it exists.
        let entry = mgr.get("test-entry").unwrap();
        assert_eq!(entry.title, "Test Entry");
        assert!(entry.content.contains("Go programming"));

        // Verify file was written.
        let path = dir.path().join("entries").join("test-entry.md");
        let data = std::fs::read_to_string(&path).unwrap();
        assert!(data.contains("Go programming"));

        // Reload and verify persistence.
        let mgr2 = Manager::new(dir.path());
        mgr2.load().await.unwrap();
        let entry2 = mgr2.get("test-entry").unwrap();
        assert_eq!(entry2.title, "Test Entry");
    }

    #[tokio::test]
    async fn test_memory_manager_search() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = Manager::new(dir.path());
        mgr.load().await.unwrap();

        mgr.save("golang", "# Go Programming\n\nGo is a statically typed, compiled language designed at Google.")
            .await
            .unwrap();
        mgr.save("python", "# Python Programming\n\nPython is a high-level interpreted programming language.")
            .await
            .unwrap();
        mgr.save("recipes", "# Favorite Recipes\n\nChocolate cake recipe with vanilla frosting.")
            .await
            .unwrap();

        let results = mgr.search("programming language", 5).await;
        assert!(!results.is_empty());

        let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&"golang") || ids.contains(&"python"));
    }

    #[tokio::test]
    async fn test_memory_manager_delete() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = Manager::new(dir.path());
        mgr.load().await.unwrap();

        mgr.save("to-delete", "# Delete Me\n\nThis will be deleted.")
            .await
            .unwrap();
        assert!(mgr.get("to-delete").is_some());

        mgr.delete("to-delete").await.unwrap();
        assert!(mgr.get("to-delete").is_none());

        // File should be gone.
        let path = dir.path().join("entries").join("to-delete.md");
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn test_memory_manager_delete_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = Manager::new(dir.path());
        mgr.load().await.unwrap();

        let result = mgr.delete("nonexistent").await;
        assert!(result.is_err());
    }

    #[test]
    fn test_format_memory_for_prompt() {
        let entries = vec![Entry {
            id: "test".to_string(),
            title: "Test Entry".to_string(),
            content: "Content here".to_string(),
            file_path: PathBuf::new(),
            mod_time: chrono::Utc::now(),
        }];

        let result = format_for_prompt(&entries);
        assert!(result.contains("## Relevant Memory"));
        assert!(result.contains("### Test Entry"));
        assert!(result.contains("Content here"));
    }

    #[test]
    fn test_format_memory_for_prompt_empty() {
        let result = format_for_prompt(&[]);
        assert_eq!(result, "");
    }

    #[test]
    fn test_format_memory_for_prompt_truncation() {
        // Create an entry with > 2000 chars.
        let long_content = "x".repeat(2100);

        let entries = vec![Entry {
            id: "long".to_string(),
            title: "Long Entry".to_string(),
            content: long_content,
            file_path: PathBuf::new(),
            mod_time: chrono::Utc::now(),
        }];

        let result = format_for_prompt(&entries);
        assert!(result.contains("[truncated]"));
    }

    // --- FormatIndex tests ---

    #[test]
    fn test_format_index_empty_manager_returns_empty() {
        let m = Manager::new(tempfile::tempdir().unwrap().path());
        assert_eq!(m.format_index(), "");
    }

    #[test]
    fn test_format_index_lists_entries_sorted_by_id() {
        let dir = tempfile::tempdir().unwrap();
        let m = Manager::new(dir.path());

        m.insert_entry_for_test(make_entry("zebra", "# Z\n\nlast line about zebras."));
        m.insert_entry_for_test(make_entry("apple", "# A\n\nfirst line about apples."));
        m.insert_entry_for_test(make_entry("banana", "# B\n\nbody about bananas."));

        let got = m.format_index();
        assert!(got.contains("## Memory Index"));

        let a = got.find("**apple**").expect("apple missing");
        let b = got.find("**banana**").expect("banana missing");
        let z = got.find("**zebra**").expect("zebra missing");
        assert!(a < b && b < z, "entries must be sorted; positions a={} b={} z={}", a, b, z);
    }

    #[test]
    fn test_format_index_includes_title_and_description() {
        let dir = tempfile::tempdir().unwrap();
        let m = Manager::new(dir.path());
        m.insert_entry_for_test(make_entry(
            "e1",
            "# First entry title\n\nThis is the one-line description.",
        ));

        let got = m.format_index();
        assert!(got.contains("**e1**"));
        assert!(got.contains("First entry title"));
        assert!(got.contains("This is the one-line description."));
    }

    #[test]
    fn test_format_index_skips_title_in_description() {
        let dir = tempfile::tempdir().unwrap();
        let m = Manager::new(dir.path());
        m.insert_entry_for_test(make_entry("e1", "# Has only title\n"));

        let got = m.format_index();
        assert!(got.contains("**e1**"));
        assert!(got.contains("Has only title"));
        // No description beyond the title.
        assert!(!got.contains(": # "));
    }

    #[test]
    fn test_format_index_caps_at_max() {
        let dir = tempfile::tempdir().unwrap();
        let m = Manager::new(dir.path());
        for i in 1..=250usize {
            let id = format!("e_{:03}", i);
            m.insert_entry_for_test(make_entry(&id, ""));
        }

        let got = m.format_index();
        assert!(got.contains("**e_001**"));
        assert!(got.contains("**e_200**"));
        assert!(!got.contains("**e_201**"));
        assert!(!got.contains("**e_250**"));
    }

    #[test]
    fn test_format_index_trims_long_description() {
        let dir = tempfile::tempdir().unwrap();
        let m = Manager::new(dir.path());
        let long = "x".repeat(500);
        let content = format!("# T\n{}", long);
        m.insert_entry_for_test(make_entry("e1", &content));

        let got = m.format_index();
        // 120-char cap + ellipsis.
        assert!(got.contains('\u{2026}'), "should contain ellipsis");
        // Must not contain the full 500-char string.
        assert!(!got.contains(&long));
    }
}
