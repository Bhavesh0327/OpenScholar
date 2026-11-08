use strsim::levenshtein;
use crate::layers::PaperMetadata;

pub struct Resolver;

impl Resolver {
    pub fn resolve(query_title: &str, candidates: Vec<PaperMetadata>, threshold: usize) -> Vec<(PaperMetadata, usize)> {
        if query_title.is_empty() {
             // If no title provided (e.g. university search), return all candidates with 0 distance
             return candidates.into_iter().map(|p| (p, 0)).collect();
        }

        let query_title = Self::normalize(query_title);
        candidates.into_iter()
            .map(|p| {
                let dist = levenshtein(&query_title, &Self::normalize(&p.title));
                tracing::debug!("Candidate: {} (Distance: {})", p.title, dist);
                (p, dist)
            })
            .filter(|(_, dist)| *dist <= threshold)
            .collect()
    }

    /// Lowercase, with punctuation and whitespace runs collapsed to single spaces.
    pub fn normalize(title: &str) -> String {
        title.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn sort_by_similarity(mut matches: Vec<(PaperMetadata, usize)>) -> Vec<(PaperMetadata, usize)> {
        matches.sort_by_key(|(_, dist)| *dist);
        matches
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to create a dummy PaperMetadata
    fn create_dummy_paper(title: &str) -> PaperMetadata {
        PaperMetadata {
            id: String::new(),
            title: title.to_string(),
            authors: vec![],
            year: None,
            doi: None,
            arxiv_id: None,
            semantic_scholar_id: None,
            open_alex_id: None,
            venue: None,
            abstract_text: None,
            pdf_url: None,
            is_oa: false,
            categories: vec![],
            sources: vec![],
        }
    }

    #[test]
    fn test_resolve_exact_match() {
        let p1 = create_dummy_paper("Quantum Computing");
        let p2 = create_dummy_paper("Introduction to ML");
        let candidates = vec![p1.clone(), p2.clone()];

        let results = Resolver::resolve("Quantum Computing", candidates, 5);
        
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.title, "Quantum Computing");
        assert_eq!(results[0].1, 0); // Distance should be 0 for exact match
    }

    #[test]
    fn test_resolve_fuzzy_match() {
        let p1 = create_dummy_paper("Quantum Computing");
        let candidates = vec![p1];

        // "Quantumm Computin" -> Typo
        let results = Resolver::resolve("Quantumm Computin", candidates, 5);
        
        assert_eq!(results.len(), 1);
        assert!(results[0].1 > 0);
        assert!(results[0].1 <= 5);
    }

    #[test]
    fn test_resolve_no_match() {
        let p1 = create_dummy_paper("Biology 101");
        let candidates = vec![p1];

        let results = Resolver::resolve("Quantum Mechanics", candidates, 2);
        
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_resolve_ignores_case_and_punctuation() {
        let candidates = vec![create_dummy_paper("Attention is All you Need!"), create_dummy_paper("BERT: Pre-training of Deep Bidirectional Transformers")];

        assert_eq!(Resolver::resolve("attention is all you need", candidates.clone(), 0).len(), 1);
        assert_eq!(Resolver::resolve("BERT Pre-Training of deep bidirectional transformers", candidates, 0).len(), 1);
    }

    #[test]
    fn test_sort_by_similarity() {
        let p1 = create_dummy_paper("A");
        let p2 = create_dummy_paper("B");
        
        // Unsorted: dist 10 then dist 2
        let matches = vec![(p1.clone(), 10), (p2.clone(), 2)];
        
        let sorted = Resolver::sort_by_similarity(matches);
        
        assert_eq!(sorted[0].1, 2);
        assert_eq!(sorted[1].1, 10);
        assert_eq!(sorted[0].0.title, "B");
    }
}
