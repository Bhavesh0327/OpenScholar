use crate::layers::PaperMetadata;
use std::collections::HashMap;

pub fn normalize_doi(raw: &str) -> Option<String> {
    let doi = raw.trim().to_lowercase();
    let doi = ["https://doi.org/", "http://doi.org/", "https://dx.doi.org/", "http://dx.doi.org/", "doi:"]
        .iter()
        .find_map(|p| doi.strip_prefix(p))
        .unwrap_or(&doi)
        .to_string();
    doi.starts_with("10.").then_some(doi)
}

pub fn normalize_arxiv_id(raw: &str) -> Option<String> {
    let id = raw.trim();
    let id = id.rsplit_once("/abs/").map(|(_, rest)| rest).unwrap_or(id);
    let id = if id.to_lowercase().starts_with("arxiv:") { &id[6..] } else { id };
    let id = match id.rfind('v') {
        Some(i) if i + 1 < id.len() && id[i + 1..].chars().all(|c| c.is_ascii_digit()) => &id[..i],
        _ => id,
    };
    (!id.is_empty()).then(|| id.to_string())
}

/// Normalizes provider IDs in place and sets `id` to the strongest available identifier.
pub fn canonicalize(mut p: PaperMetadata) -> PaperMetadata {
    p.doi = p.doi.as_deref().and_then(normalize_doi);
    p.arxiv_id = p.arxiv_id.as_deref().and_then(normalize_arxiv_id)
        .or_else(|| p.doi.as_deref().and_then(|d| d.strip_prefix("10.48550/arxiv.")).map(String::from));
    p.open_alex_id = p.open_alex_id.map(|id| id.rsplit('/').next().unwrap_or(&id).to_string());
    p.id = keys(&p).into_iter().next().unwrap_or_default();
    p
}

fn keys(p: &PaperMetadata) -> Vec<String> {
    [
        p.doi.as_ref().map(|v| format!("doi:{v}")),
        p.arxiv_id.as_ref().map(|v| format!("arxiv:{v}")),
        p.semantic_scholar_id.as_ref().map(|v| format!("s2:{v}")),
        p.open_alex_id.as_ref().map(|v| format!("openalex:{v}")),
    ].into_iter().flatten().collect()
}

/// Merges papers that share any identifier, keeping first-seen order and filling gaps from later copies.
pub fn dedupe(papers: Vec<PaperMetadata>) -> Vec<PaperMetadata> {
    let mut merged: Vec<PaperMetadata> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();

    for p in papers {
        let p = canonicalize(p);
        let p_keys = keys(&p);
        let Some(i) = p_keys.iter().find_map(|k| index.get(k).copied()) else {
            for k in p_keys { index.insert(k, merged.len()); }
            merged.push(p);
            continue;
        };

        let m = &mut merged[i];
        m.doi = m.doi.take().or(p.doi);
        m.arxiv_id = m.arxiv_id.take().or(p.arxiv_id);
        m.semantic_scholar_id = m.semantic_scholar_id.take().or(p.semantic_scholar_id);
        m.open_alex_id = m.open_alex_id.take().or(p.open_alex_id);
        m.year = m.year.or(p.year);
        m.venue = m.venue.take().or(p.venue);
        m.abstract_text = m.abstract_text.take().or(p.abstract_text);
        if p.authors.len() > m.authors.len() { m.authors = p.authors; }
        if !(m.is_oa && m.pdf_url.is_some()) && p.is_oa && p.pdf_url.is_some() {
            m.is_oa = true;
            m.pdf_url = p.pdf_url;
        }
        for s in p.sources { if !m.sources.contains(&s) { m.sources.push(s); } }

        let m = canonicalize(m.clone());
        for k in keys(&m) { index.insert(k, i); }
        merged[i] = m;
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_provider_id_forms() {
        assert_eq!(normalize_doi("https://doi.org/10.48550/ARXIV.1706.03762").as_deref(), Some("10.48550/arxiv.1706.03762"));
        assert_eq!(normalize_doi("not-a-doi"), None);
        assert_eq!(normalize_arxiv_id("http://arxiv.org/abs/1706.03762v7").as_deref(), Some("1706.03762"));
        assert_eq!(normalize_arxiv_id("arXiv:hep-th/9901001v2").as_deref(), Some("hep-th/9901001"));
    }
}
