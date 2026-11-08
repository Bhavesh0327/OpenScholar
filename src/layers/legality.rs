// Basic legality checker for Open Access
use crate::layers::PaperMetadata;

pub struct LegalityChecker;

impl LegalityChecker {
    pub fn is_legally_downloadable(paper: &PaperMetadata) -> bool {
        // Basic check for Open Access
        paper.is_oa
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers::PaperMetadata;

    fn create_paper(is_oa: bool) -> PaperMetadata {
         PaperMetadata {
            id: String::new(),
            title: "Test".to_string(),
            authors: vec![],
            year: None,
            doi: None,
            arxiv_id: None,
            semantic_scholar_id: None,
            open_alex_id: None,
            venue: None,
            abstract_text: None,
            pdf_url: None,
            is_oa,
            categories: vec![],
            sources: vec![],
        }
    }

    #[test]
    fn test_is_legally_downloadable_true() {
        let paper = create_paper(true);
        assert!(LegalityChecker::is_legally_downloadable(&paper));
    }

    #[test]
    fn test_is_legally_downloadable_false() {
        let paper = create_paper(false);
        assert!(!LegalityChecker::is_legally_downloadable(&paper));
    }
}
