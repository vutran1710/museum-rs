//! Splitting slash-separated addresses (`owner/repo`, `<ref>/<path>`) into segments, shared by the
//! forge stores; each forge turns a refusal into its own error.

/// The address's segments, or `None` when any of them is empty.
pub(crate) fn segments(address: &str) -> Option<Vec<&str>> {
    let segments: Vec<&str> = address.trim_matches('/').split('/').collect();
    segments
        .iter()
        .all(|segment| !segment.is_empty())
        .then_some(segments)
}

/// `<ref>/<path>`: the first segment, and the rest joined back, which must not be empty.
pub(crate) fn reference_and_path(address: &str) -> Option<(String, String)> {
    match &segments(address)?[..] {
        [reference, path @ ..] if !path.is_empty() => {
            Some(((*reference).to_owned(), path.join("/")))
        }
        _ => None,
    }
}
