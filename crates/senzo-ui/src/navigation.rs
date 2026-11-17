//! Small, deterministic route history independent of GPUI rendering.

use senzo_domain::AppPage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NavigationStack {
    back: Vec<AppPage>,
    forward: Vec<AppPage>,
}

impl Default for NavigationStack {
    fn default() -> Self {
        Self::new()
    }
}

impl NavigationStack {
    pub(crate) fn new() -> Self {
        Self {
            back: vec![AppPage::Home],
            forward: Vec::new(),
        }
    }

    pub(crate) fn current(&self) -> AppPage {
        *self.back.last().expect("navigation always has a root route")
    }

    pub(crate) fn can_go_back(&self) -> bool {
        self.back.len() > 1
    }

    pub(crate) fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    pub(crate) fn depth(&self) -> usize {
        self.back.len()
    }

    pub(crate) fn navigate(&mut self, page: AppPage) -> bool {
        if self.current() == page {
            return false;
        }

        self.back.push(page);
        self.forward.clear();
        true
    }

    pub(crate) fn back(&mut self) -> bool {
        if !self.can_go_back() {
            return false;
        }

        if let Some(page) = self.back.pop() {
            self.forward.push(page);
            true
        } else {
            false
        }
    }

    pub(crate) fn forward(&mut self) -> bool {
        let Some(page) = self.forward.pop() else {
            return false;
        };

        self.back.push(page);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_history_supports_back_and_forward() {
        let mut routes = NavigationStack::new();
        assert_eq!(routes.current(), AppPage::Home);

        assert!(routes.navigate(AppPage::Movies));
        assert!(routes.navigate(AppPage::TvShows));
        assert_eq!(routes.depth(), 3);
        assert!(routes.back());
        assert_eq!(routes.current(), AppPage::Movies);
        assert!(routes.forward());
        assert_eq!(routes.current(), AppPage::TvShows);
    }

    #[test]
    fn navigating_after_back_discards_forward_history() {
        let mut routes = NavigationStack::new();
        routes.navigate(AppPage::Movies);
        routes.navigate(AppPage::Library);
        assert!(routes.back());
        assert!(routes.navigate(AppPage::Search));
        assert!(!routes.can_go_forward());
        assert_eq!(routes.current(), AppPage::Search);
    }

    #[test]
    fn duplicate_route_does_not_create_history_entries() {
        let mut routes = NavigationStack::new();
        assert!(!routes.navigate(AppPage::Home));
        assert_eq!(routes.depth(), 1);
        assert!(!routes.back());
    }
}
