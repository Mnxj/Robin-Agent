/// BM25 parameters
const BM25_K1: f64 = 1.2;
const BM25_B: f64 = 0.75;

/// A single document stored in the BM25 index.
struct IndexedDoc {
    id: String,
    terms: std::collections::HashMap<String, usize>,
    length: usize,
}

/// In-process BM25 search index.
pub struct Bm25Index {
    docs: Vec<IndexedDoc>,
    avg_dl: f64,
    df: std::collections::HashMap<String, usize>,
    doc_count: usize,
}

/// A search result with a relevance score.
pub struct SearchResult {
    pub id: String,
    pub score: f64,
}

impl Bm25Index {
    /// Creates an empty BM25 index.
    pub fn new() -> Self {
        Self {
            docs: Vec::new(),
            avg_dl: 0.0,
            df: std::collections::HashMap::new(),
            doc_count: 0,
        }
    }

    /// Indexes a document with the given ID and text content.
    pub fn add(&mut self, id: &str, text: &str) {
        let tokens = tokenize(text);
        let mut tf: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for tok in &tokens {
            *tf.entry(tok.clone()).or_insert(0) += 1;
        }

        // Track unique terms for document frequency.
        for term in tf.keys() {
            *self.df.entry(term.clone()).or_insert(0) += 1;
        }

        self.docs.push(IndexedDoc {
            id: id.to_string(),
            terms: tf,
            length: tokens.len(),
        });
        self.doc_count += 1;

        // Recompute average document length.
        let total_len: usize = self.docs.iter().map(|d| d.length).sum();
        self.avg_dl = total_len as f64 / self.doc_count as f64;
    }

    /// Returns document IDs ranked by BM25 relevance to the query.
    pub fn search(&self, query: &str, max_results: usize) -> Vec<SearchResult> {
        if self.doc_count == 0 {
            return Vec::new();
        }

        let query_terms = tokenize(query);
        if query_terms.is_empty() {
            return Vec::new();
        }

        let mut results: Vec<(String, f64)> = Vec::new();

        for doc in &self.docs {
            let mut score = 0.0f64;
            for term in &query_terms {
                let tf = *doc.terms.get(term).unwrap_or(&0) as f64;
                if tf == 0.0 {
                    continue;
                }

                let df = *self.df.get(term).unwrap_or(&0) as f64;
                // IDF with smoothing.
                let idf = (1.0 + (self.doc_count as f64 - df + 0.5) / (df + 0.5)).ln();

                // BM25 TF normalization.
                let tf_norm = (tf * (BM25_K1 + 1.0))
                    / (tf + BM25_K1 * (1.0 - BM25_B + BM25_B * doc.length as f64 / self.avg_dl));

                score += idf * tf_norm;
            }

            if score > 0.0 {
                results.push((doc.id.clone(), score));
            }
        }

        // Sort by score descending.
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        if max_results > 0 && results.len() > max_results {
            results.truncate(max_results);
        }

        results
            .into_iter()
            .map(|(id, score)| SearchResult { id, score })
            .collect()
    }
}

impl Default for Bm25Index {
    fn default() -> Self {
        Self::new()
    }
}

/// Splits text into lowercase word tokens, removing punctuation.
/// Single-character tokens are filtered out.
pub(crate) fn tokenize(text: &str) -> Vec<String> {
    let text = text.to_lowercase();
    let mut tokens = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        if ch.is_alphabetic() || ch.is_ascii_digit() {
            current.push(ch);
        } else if !current.is_empty() {
            if current.len() >= 2 {
                tokens.push(current.clone());
            }
            current.clear();
        }
    }
    if current.len() >= 2 {
        tokens.push(current);
    }

    tokens
}

#[cfg(test)]
#[path = "bm25_test.rs"]
mod tests;