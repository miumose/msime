//! Candidate glosses were removed with the translation feature; the runtime keeps
//! the same shape so the candidate document does not need a second edit.

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TranslationRun {
    pub text: String,
}

pub fn source_gets_no_gloss(_source: usize) -> bool {
    true
}

pub fn translation_runs(_text: &str) -> Vec<TranslationRun> {
    Vec::new()
}
