#[cfg(test)]
mod tests {
    use crate::memory::bm25::{Bm25Index, tokenize};

    #[test]
    fn test_bm25_index_basic() {
        let mut idx = Bm25Index::new();
        idx.add("doc1", "the quick brown fox jumps over the lazy dog");
        idx.add("doc2", "machine learning and artificial intelligence");
        idx.add("doc3", "the quick brown fox and the lazy cat");

        let results = idx.search("quick fox", 5);
        assert!(!results.is_empty());
        // doc1 and doc3 mention both "quick" and "fox"
        let top_id = &results[0].id;
        assert!(top_id == "doc1" || top_id == "doc3");
    }

    #[test]
    fn test_bm25_index_empty() {
        let idx = Bm25Index::new();
        let results = idx.search("test", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_bm25_index_no_match() {
        let mut idx = Bm25Index::new();
        idx.add("doc1", "hello world");
        let results = idx.search("quantum physics", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_bm25_index_max_results() {
        let mut idx = Bm25Index::new();
        for i in 0..10usize {
            idx.add(&format!("doc{}", i), "test document about golang programming");
        }
        let results = idx.search("golang", 3);
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_tokenize() {
        let tokens = tokenize("Hello, World! This is a test 123.");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(tokens.contains(&"this".to_string()));
        assert!(tokens.contains(&"test".to_string()));
        assert!(tokens.contains(&"123".to_string()));
        // Single-char tokens should be filtered.
        assert!(!tokens.contains(&"a".to_string()));
    }
}