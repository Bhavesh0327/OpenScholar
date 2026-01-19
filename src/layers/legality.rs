// Basic legality checker for Open Access
use crate::layers::PaperMetadata;

pub struct LegalityChecker;

impl LegalityChecker {
    pub fn is_legally_downloadable(paper: &PaperMetadata) -> bool {
        // Basic check for Open Access
        paper.is_oa
    }
}
