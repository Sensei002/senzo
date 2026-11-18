use senzo_metadata::MediaType;
pub(crate) const SEARCH_HISTORY_LIMIT: usize = 10;
pub(crate) const SEARCH_HISTORY_MAX_BYTES: usize = 8 * 1024;

pub(crate) fn parse_search_history(payload: Option<&str>) -> Vec<String> {
    let Some(payload) = payload.filter(|value| value.len() <= SEARCH_HISTORY_MAX_BYTES) else { return Vec::new(); };
    let Ok(entries) = serde_json::from_str::<Vec<String>>(payload) else { return Vec::new(); };
    let mut seen = std::collections::HashSet::new();
    let mut history = Vec::new();
    for entry in entries {
        let query = entry.trim();
        if query.chars().count() < 2 || query.chars().count() > 120 || query.chars().any(char::is_control) { continue; }
        if seen.insert(query.to_lowercase()) {
            history.push(query.to_owned());
            if history.len() == SEARCH_HISTORY_LIMIT { break; }
        }
    }
    history
}

pub(crate) fn record_search_history(current: &[String], query: &str) -> Vec<String> {
    let query = query.trim();
    if query.chars().count() < 2 || query.chars().count() > 120 || query.chars().any(char::is_control) {
        return current.to_vec();
    }
    let mut updated = Vec::with_capacity(SEARCH_HISTORY_LIMIT);
    updated.push(query.to_owned());
    for existing in current {
        if !existing.eq_ignore_ascii_case(query) && updated.len() < SEARCH_HISTORY_LIMIT { updated.push(existing.clone()); }
    }
    updated
}

pub(crate) fn remove_search_history_entry(current: &[String], query: &str) -> Vec<String> {
    current.iter().filter(|entry| !entry.eq_ignore_ascii_case(query)).cloned().collect()
}

pub(crate) const fn media_identity(media_type: MediaType, id: u64) -> (u8, u64) {
    let kind = match media_type {
        MediaType::Movie => 1, MediaType::Tv => 2, MediaType::Person => 3, MediaType::Other => 4,
    };
    (kind, id)
}

pub(crate) fn season_label(number: u32, name: &str) -> String {
    if number == 0 { "Specials".to_owned() } else if name.trim().is_empty() { format!("Season {number}") } else { name.to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_is_trimmed_deduplicated_and_bounded() {
        let entries=(0..15).map(|i|format!("Title {i}")).collect::<Vec<_>>();
        let parsed=parse_search_history(Some(&serde_json::to_string(&entries).unwrap()));
        assert_eq!(parsed.len(),SEARCH_HISTORY_LIMIT);
        assert_eq!(parsed[0],"Title 0");
    }
    #[test]
    fn history_rejects_bad_payloads_and_short_terms() {
        assert!(parse_search_history(Some("{")).is_empty());
        assert!(parse_search_history(Some(&"x".repeat(SEARCH_HISTORY_MAX_BYTES+1))).is_empty());
        assert_eq!(parse_search_history(Some(r#"["a","  "," Dune ","dune","The Bear"]"#)),vec!["Dune","The Bear"]);
    }
    #[test]
    fn recording_moves_existing_terms_to_front_without_case_duplicates() {
        assert_eq!(record_search_history(&["Dune".into(),"The Bear".into()]," dune "),vec!["dune","The Bear"]);
    }
    #[test]
    fn media_identity_keeps_movie_and_tv_ids_separate() {
        assert_ne!(media_identity(MediaType::Movie,42),media_identity(MediaType::Tv,42));
    }
    #[test]
    fn season_zero_is_presented_as_specials() {
        assert_eq!(season_label(0,"Season 0"),"Specials");
        assert_eq!(season_label(2,"Season 2"),"Season 2");
        assert_eq!(season_label(3,""),"Season 3");
    }
}
