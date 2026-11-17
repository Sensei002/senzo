//! Native SENZO desktop shell: routes, focus, menus, dialogs and window actions.

use std::{collections::HashMap, sync::Arc, time::Duration};

use gpui::{
    App, Bounds, ColorExt, Context, Entity, FocusHandle, Image, ImageFormat, KeyBinding, Menu,
    MenuItem, MouseButton, NavigationDirection, QuitMode, Render, RenderImage, SharedString,
    Subscription, TitlebarOptions, Window, WindowBounds, WindowOptions, actions, div, img,
    prelude::*, px, rgb, size,
};
use gpui_component::input::{Input, InputEvent, InputState};
use senzo_artwork::{ArtworkService, ArtworkSize, ArtworkUrlResolver};
use senzo_domain::AppPage;
use senzo_metadata::{MediaType, MetadataClient, SearchType};
use tokio::{runtime::Handle, task::JoinSet};
use tokio_util::sync::CancellationToken;

use crate::{
    artwork::{backdrop_surface, poster_card, render_image_from_artwork},
    components::{StatusTone, divider, feature_tile, page_intro, status_panel},
    navigation::NavigationStack,
    theme::Theme,
};

actions!(
    senzo,
    [
        GoBack,
        GoForward,
        ToggleFullscreen,
        FocusNext,
        FocusPrevious,
        ActivateNavigation,
        MoveNavigationUp,
        MoveNavigationDown,
        ShowShortcuts,
        ShowAbout,
        ShowWhatsNew,
        Quit
    ]
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DialogKind {
    About,
    Shortcuts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HomeCatalogueStatus {
    NotRequested,
    Loading,
    Loaded,
}

#[derive(Clone)]
struct HomeMediaCandidate {
    title: String,
    detail: String,
    poster_path: String,
    backdrop_path: Option<String>,
}

struct HomeMediaCard {
    title: String,
    detail: String,
    poster: Option<Arc<RenderImage>>,
}

struct HomeCatalogueData {
    cards: Vec<HomeMediaCard>,
    featured_backdrop: Option<Arc<RenderImage>>,
    featured_title: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchFilter {
    All,
    Movies,
    TvShows,
}

impl SearchFilter {
    const fn search_type(self) -> SearchType {
        match self {
            Self::All => SearchType::Multi,
            Self::Movies => SearchType::Movie,
            Self::TvShows => SearchType::Tv,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Movies => "Movies",
            Self::TvShows => "TV Shows",
        }
    }

    const fn key(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Movies => "movies",
            Self::TvShows => "tv-shows",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchStatus {
    Idle,
    Debouncing,
    Loading,
    Results,
    Empty,
    Failed,
}

struct SearchMediaCard {
    id: u64,
    media_type: MediaType,
    title: String,
    detail: String,
    overview: Option<String>,
    vote_average: Option<f64>,
    poster: Option<Arc<RenderImage>>,
}

pub(crate) struct SenzoShell {
    navigation: NavigationStack,
    navigation_focus: HashMap<AppPage, FocusHandle>,
    dialog_focus: FocusHandle,
    tokio: Handle,
    metadata: MetadataClient,
    artwork: ArtworkService,
    home_cancellation: CancellationToken,
    home_catalogue_status: HomeCatalogueStatus,
    home_catalogue_error: Option<String>,
    home_cards: Vec<HomeMediaCard>,
    home_featured_backdrop: Option<Arc<RenderImage>>,
    home_featured_title: Option<String>,
    search_input: Entity<InputState>,
    _search_input_subscription: Subscription,
    search_filter: SearchFilter,
    search_query: String,
    search_status: SearchStatus,
    search_error: Option<String>,
    search_results: Vec<SearchMediaCard>,
    search_total_results: Option<u32>,
    selected_search_result: Option<usize>,
    search_artwork_loading: bool,
    search_cancellation: CancellationToken,
    search_generation: u64,
    logo: Arc<Image>,
    dialog: Option<DialogKind>,
    menu_open: bool,
    notification: Option<String>,
    runtime_status: String,
    smoke_test: bool,
    smoke_test_scheduled: bool,
}

impl SenzoShell {
    pub(crate) fn new(
        tokio: Handle,
        smoke_test: bool,
        metadata: MetadataClient,
        artwork: ArtworkService,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let navigation_focus = AppPage::ALL
            .iter()
            .enumerate()
            .map(|(index, page)| (*page, cx.focus_handle().tab_index(index as isize)))
            .collect();
        let dialog_focus = cx.focus_handle().tab_index(60);
        let search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search movies, TV shows, and people")
        });
        let search_input_subscription = cx.subscribe(&search_input, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.update_search_query(input.read(cx).value().to_string(), cx);
            }
        });

        Self {
            navigation: NavigationStack::new(),
            navigation_focus,
            dialog_focus,
            tokio,
            metadata,
            artwork,
            home_cancellation: CancellationToken::new(),
            home_catalogue_status: HomeCatalogueStatus::NotRequested,
            home_catalogue_error: None,
            home_cards: Vec::new(),
            home_featured_backdrop: None,
            home_featured_title: None,
            search_input,
            _search_input_subscription: search_input_subscription,
            search_filter: SearchFilter::All,
            search_query: String::new(),
            search_status: SearchStatus::Idle,
            search_error: None,
            search_results: Vec::new(),
            search_total_results: None,
            selected_search_result: None,
            search_artwork_loading: false,
            search_cancellation: CancellationToken::new(),
            search_generation: 0,
            logo: Arc::new(Image::from_bytes(
                ImageFormat::Png,
                include_bytes!("../../../assets/branding/senzo-logo.png").to_vec(),
            )),
            dialog: None,
            menu_open: false,
            notification: None,
            runtime_status: "Not checked yet".to_owned(),
            smoke_test,
            smoke_test_scheduled: false,
        }
    }

    fn focus_page(&self, page: AppPage, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(handle) = self.navigation_focus.get(&page) {
            window.focus(handle, cx);
        }
    }

    fn navigate(&mut self, page: AppPage, window: &mut Window, cx: &mut Context<Self>) {
        if self.navigation.navigate(page) {
            self.menu_open = false;
            self.focus_page(page, window, cx);
            cx.notify();
        }
    }

    fn move_nav_focus(&self, direction: isize, window: &mut Window, cx: &mut Context<Self>) {
        let current = AppPage::ALL
            .iter()
            .position(|page| {
                self.navigation_focus
                    .get(page)
                    .is_some_and(|handle| handle.is_focused(window))
            })
            .unwrap_or_else(|| {
                AppPage::ALL
                    .iter()
                    .position(|page| *page == self.navigation.current())
                    .unwrap_or(0)
            });
        let next = (current as isize + direction).rem_euclid(AppPage::ALL.len() as isize) as usize;
        self.focus_page(AppPage::ALL[next], window, cx);
    }

    fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.take().is_some() {
            let page = self.navigation.current();
            self.focus_page(page, window, cx);
            cx.notify();
            return;
        }
        if self.menu_open {
            self.menu_open = false;
            let page = self.navigation.current();
            self.focus_page(page, window, cx);
            cx.notify();
            return;
        }
        if self.navigation.back() {
            let page = self.navigation.current();
            self.focus_page(page, window, cx);
            cx.notify();
        }
    }

    fn go_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.is_some() || self.menu_open {
            return;
        }
        if self.navigation.can_go_forward() && self.navigation.forward() {
            let page = self.navigation.current();
            self.focus_page(page, window, cx);
            cx.notify();
        }
    }

    fn open_dialog(&mut self, kind: DialogKind, window: &mut Window, cx: &mut Context<Self>) {
        self.menu_open = false;
        self.dialog = Some(kind);
        window.focus(&self.dialog_focus, cx);
        cx.notify();
    }

    fn check_runtime(&mut self, cx: &mut Context<Self>) {
        let runtime = self.tokio.clone();
        self.runtime_status = "Running a background task…".to_owned();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn(async {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    "Background runtime is responding"
                })
                .await;
            let status = match result {
                Ok(message) => message.to_owned(),
                Err(error) => format!("Background task failed: {error}"),
            };
            let _ = this.update(cx, |this, cx| {
                this.runtime_status = status;
                cx.notify();
            });
        })
        .detach();
    }

    fn load_home_catalogue(&mut self, cx: &mut Context<Self>) {
        if self.home_catalogue_status == HomeCatalogueStatus::Loading
            || self.home_catalogue_status == HomeCatalogueStatus::Loaded
        {
            return;
        }

        self.home_catalogue_status = HomeCatalogueStatus::Loading;
        self.home_catalogue_error = None;

        let metadata = self.metadata.clone();
        let artwork = self.artwork.clone();
        let cancellation = self.home_cancellation.child_token();
        let task = self.tokio.spawn(async move {
            let response = metadata
                .trending(1, "en-US", &cancellation)
                .await
                .map_err(|_| {
                    "The trending catalogue is unavailable. Check your connection and try again."
                        .to_owned()
                })?;

            let candidates: Vec<HomeMediaCandidate> = response
                .results
                .into_iter()
                .filter_map(|media| {
                    let poster_path = media.poster_path.clone()?;
                    let category = match media.media_type.unwrap_or(MediaType::Other) {
                        MediaType::Movie => "MOVIE",
                        MediaType::Tv => "SERIES",
                        MediaType::Person => "PERSON",
                        MediaType::Other => "TRENDING",
                    };
                    let detail = media
                        .release_year()
                        .map(|year| format!("{category} · {year}"))
                        .unwrap_or_else(|| category.to_owned());
                    Some(HomeMediaCandidate {
                        title: media.display_title().to_owned(),
                        detail,
                        poster_path,
                        backdrop_path: media.backdrop_path,
                    })
                })
                .take(8)
                .collect();

            let featured_index = candidates
                .iter()
                .position(|candidate| candidate.backdrop_path.is_some());
            let mut cards: Vec<HomeMediaCard> = candidates
                .iter()
                .map(|candidate| HomeMediaCard {
                    title: candidate.title.clone(),
                    detail: candidate.detail.clone(),
                    poster: None,
                })
                .collect();

            let mut jobs = JoinSet::new();
            for (index, candidate) in candidates.into_iter().enumerate() {
                let artwork = artwork.clone();
                let cancellation = cancellation.child_token();
                jobs.spawn(async move {
                    let poster = load_gpui_artwork(
                        artwork.clone(),
                        candidate.poster_path,
                        ArtworkSize::W342,
                        cancellation.child_token(),
                    )
                    .await;
                    let backdrop = if Some(index) == featured_index {
                        match candidate.backdrop_path {
                            Some(path) => {
                                load_gpui_artwork(artwork, path, ArtworkSize::W1280, cancellation)
                                    .await
                            }
                            None => None,
                        }
                    } else {
                        None
                    };
                    (index, poster, backdrop)
                });
            }

            let mut featured_backdrop = None;
            while let Some(result) = jobs.join_next().await {
                if let Ok((index, poster, backdrop)) = result {
                    if let Some(card) = cards.get_mut(index) {
                        card.poster = poster;
                    }
                    if Some(index) == featured_index {
                        featured_backdrop = backdrop;
                    }
                }
            }

            let featured_title = featured_index
                .and_then(|index| cards.get(index).map(|card| card.title.clone()))
                .or_else(|| cards.first().map(|card| card.title.clone()));

            Ok::<_, String>(HomeCatalogueData {
                cards,
                featured_backdrop,
                featured_title,
            })
        });

        cx.spawn(async move |this, cx| {
            let result = match task.await {
                Ok(result) => result,
                Err(_) => Err("The catalogue task stopped unexpectedly.".to_owned()),
            };
            let _ = this.update(cx, |this, cx| {
                if this.home_cancellation.is_cancelled() {
                    return;
                }
                match result {
                    Ok(data) => {
                        this.home_cards = data.cards;
                        this.home_featured_backdrop = data.featured_backdrop;
                        this.home_featured_title = data.featured_title;
                        this.home_catalogue_status = HomeCatalogueStatus::Loaded;
                        this.home_catalogue_error = None;
                    }
                    Err(message) => {
                        this.home_cards.clear();
                        this.home_featured_backdrop = None;
                        this.home_featured_title = None;
                        this.home_catalogue_status = HomeCatalogueStatus::NotRequested;
                        this.home_catalogue_error = Some(message);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn update_search_query(&mut self, query: String, cx: &mut Context<Self>) {
        if self.search_query == query {
            return;
        }
        self.search_query = query;
        self.schedule_search(cx);
    }

    fn set_search_filter(&mut self, filter: SearchFilter, cx: &mut Context<Self>) {
        if self.search_filter == filter {
            return;
        }
        self.search_filter = filter;
        self.schedule_search(cx);
    }

    /// Debounce hosted search and cancel superseded metadata/artwork requests.
    fn schedule_search(&mut self, cx: &mut Context<Self>) {
        self.search_cancellation.cancel();
        self.search_generation = self.search_generation.wrapping_add(1);
        let generation = self.search_generation;
        let query = self.search_query.trim().to_owned();
        self.search_results.clear();
        self.search_total_results = None;
        self.selected_search_result = None;
        self.search_error = None;
        self.search_artwork_loading = false;

        if query.is_empty() {
            self.search_status = SearchStatus::Idle;
            self.search_cancellation = CancellationToken::new();
            cx.notify();
            return;
        }

        let cancellation = CancellationToken::new();
        self.search_cancellation = cancellation.clone();
        self.search_status = SearchStatus::Debouncing;
        let runtime = self.tokio.clone();
        let metadata = self.metadata.clone();
        let artwork = self.artwork.clone();
        let search_type = self.search_filter.search_type();
        cx.notify();

        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(350))
                .await;

            if cancellation.is_cancelled() {
                return;
            }

            let _ = this.update(cx, |this, cx| {
                if this.search_generation == generation {
                    this.search_status = SearchStatus::Loading;
                    cx.notify();
                }
            });

            let request_token = cancellation.clone();
            let response = runtime
                .spawn(async move {
                    metadata
                        .search(&query, search_type, 1, "en-US", &request_token)
                        .await
                })
                .await;

            if cancellation.is_cancelled() {
                return;
            }

            let response = match response {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => {
                    let message = error.to_string();
                    let _ = this.update(cx, |this, cx| {
                        if this.search_generation == generation && !cancellation.is_cancelled() {
                            this.search_status = SearchStatus::Failed;
                            this.search_error = Some(message);
                            this.search_artwork_loading = false;
                            cx.notify();
                        }
                    });
                    return;
                }
                Err(error) => {
                    let message = format!("The metadata request stopped unexpectedly: {error}");
                    let _ = this.update(cx, |this, cx| {
                        if this.search_generation == generation && !cancellation.is_cancelled() {
                            this.search_status = SearchStatus::Failed;
                            this.search_error = Some(message);
                            this.search_artwork_loading = false;
                            cx.notify();
                        }
                    });
                    return;
                }
            };

            let total_results = response.total_results;
            let mut cards = Vec::new();
            let mut image_requests = Vec::new();

            for media in response.results.into_iter().take(24) {
                let media_type = media.media_type.unwrap_or(match search_type {
                    SearchType::Movie => MediaType::Movie,
                    SearchType::Tv => MediaType::Tv,
                    SearchType::Multi => MediaType::Other,
                });
                let is_person = media_type == MediaType::Person;
                let poster_path = if is_person {
                    media.profile_path.clone().or(media.poster_path.clone())
                } else {
                    media.poster_path.clone().or(media.profile_path.clone())
                };
                let image_size = if is_person {
                    ArtworkSize::W185
                } else {
                    ArtworkSize::W342
                };
                if let Some(path) = poster_path {
                    image_requests.push((cards.len(), path, image_size));
                }

                let category = match media_type {
                    MediaType::Movie => "MOVIE",
                    MediaType::Tv => "TV SERIES",
                    MediaType::Person => "PERSON",
                    MediaType::Other => "TITLE",
                };
                let detail = media
                    .release_year()
                    .map(|year| format!("{category} · {year}"))
                    .unwrap_or_else(|| category.to_owned());

                cards.push(SearchMediaCard {
                    id: media.id,
                    media_type,
                    title: media.display_title().to_owned(),
                    detail,
                    overview: media.overview,
                    vote_average: media.vote_average,
                    poster: None,
                });
            }

            let is_empty = cards.is_empty();
            let has_images = !image_requests.is_empty();
            let _ = this.update(cx, |this, cx| {
                if this.search_generation != generation || cancellation.is_cancelled() {
                    return;
                }
                this.search_results = cards;
                this.search_total_results = total_results;
                this.search_status = if is_empty {
                    SearchStatus::Empty
                } else {
                    SearchStatus::Results
                };
                this.search_artwork_loading = has_images;
                cx.notify();
            });

            if !has_images || cancellation.is_cancelled() {
                return;
            }

            let artwork_token = cancellation.clone();
            let image_results = runtime
                .spawn(async move {
                    let mut jobs = JoinSet::new();
                    for (index, path, size) in image_requests {
                        let service = artwork.clone();
                        let token = artwork_token.child_token();
                        jobs.spawn(async move {
                            let image = load_gpui_artwork(service, path, size, token).await;
                            (index, image)
                        });
                    }

                    let mut images = Vec::new();
                    while let Some(result) = jobs.join_next().await {
                        if let Ok((index, Some(image))) = result {
                            images.push((index, image));
                        }
                    }
                    images
                })
                .await;

            if cancellation.is_cancelled() {
                return;
            }

            let images = image_results.unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                if this.search_generation != generation || cancellation.is_cancelled() {
                    return;
                }
                for (index, image) in images {
                    if let Some(card) = this.search_results.get_mut(index) {
                        card.poster = Some(image);
                    }
                }
                this.search_artwork_loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn search_filter_button(
        &self,
        filter: SearchFilter,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = self.search_filter == filter;
        let tab_index = match filter {
            SearchFilter::All => 26,
            SearchFilter::Movies => 27,
            SearchFilter::TvShows => 28,
        };

        div()
            .id(format!("search-filter-{}", filter.key()))
            .tab_index(tab_index)
            .rounded_md()
            .border_1()
            .border_color(rgb(if active { Theme::ACCENT } else { Theme::BORDER }))
            .bg(rgb(if active {
                Theme::ACCENT_DARK
            } else {
                Theme::SURFACE
            }))
            .px_4()
            .py_2()
            .cursor_pointer()
            .hover(|style| style.bg(rgb(Theme::SURFACE_HOVER)))
            .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
            .aria_label(format!("Search {}", filter.label()))
            .on_click(cx.listener(move |this, _event, _window, cx| {
                this.set_search_filter(filter, cx);
            }))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(if active {
                        Theme::ACCENT
                    } else {
                        Theme::TEXT_SECONDARY
                    }))
                    .child(filter.label()),
            )
    }

    fn render_search(&self, cx: &mut Context<Self>) -> gpui::Div {
        let input = Input::new(&self.search_input)
            .tab_index(25)
            .aria_label("Search movies, TV shows, and people")
            .cleanable(true)
            .w_full()
            .bg(rgb(Theme::SURFACE))
            .border_1()
            .border_color(rgb(Theme::BORDER))
            .rounded_lg()
            .px_4()
            .py_3();

        let filters = div()
            .flex()
            .items_center()
            .gap_2()
            .child(self.search_filter_button(SearchFilter::All, cx))
            .child(self.search_filter_button(SearchFilter::Movies, cx))
            .child(self.search_filter_button(SearchFilter::TvShows, cx));

        let mut page = div()
            .w_full()
            .flex()
            .flex_col()
            .gap_5()
            .child(page_intro(
                "FIND SOMETHING",
                "Search",
                "Search the SENZO metadata catalogue for movies, TV shows, and people.",
            ))
            .child(input)
            .child(filters);

        if self.search_query.trim().is_empty() {
            return page.child(status_panel(
                StatusTone::Info,
                "READY WHEN YOU ARE",
                "Find something to watch",
                "Enter a title, series, or person's name. Search uses SENZO's hosted metadata service; no TMDB credential is stored in this app.",
            ));
        }

        match self.search_status {
            SearchStatus::Idle => {
                page = page.child(status_panel(
                    StatusTone::Info,
                    "SEARCH",
                    "Enter a search query",
                    "Search across movies, TV shows, and people.",
                ));
            }
            SearchStatus::Debouncing | SearchStatus::Loading => {
                let mut skeletons = div().w_full().flex().flex_wrap().gap_4();
                for _ in 0..6 {
                    skeletons = skeletons.child(poster_card(" ", " ", None, true));
                }
                page = page
                    .child(status_panel(
                        StatusTone::Loading,
                        "SEARCHING",
                        "Looking through the catalogue",
                        "Your latest query is being sent to the hosted metadata service.",
                    ))
                    .child(skeletons);
            }
            SearchStatus::Failed => {
                page = page.child(status_panel(
                    StatusTone::Error,
                    "SEARCH UNAVAILABLE",
                    "The catalogue could not be searched",
                    self.search_error
                        .as_deref()
                        .unwrap_or("Check your connection and try again."),
                ));
                page = page.child(
                    div()
                        .id("retry-metadata-search")
                        .tab_index(29)
                        .rounded_md()
                        .bg(rgb(Theme::ACCENT_DARK))
                        .px_3()
                        .py_2()
                        .cursor_pointer()
                        .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                        .aria_label("Retry search")
                        .on_click(cx.listener(|this, _event, _window, cx| {
                            this.schedule_search(cx);
                        }))
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(Theme::ACCENT))
                                .child("Retry"),
                        ),
                );
            }
            SearchStatus::Empty => {
                page = page.child(status_panel(
                    StatusTone::Empty,
                    "NO RESULTS",
                    "Nothing matched that search",
                    "Try another title or switch between All, Movies, and TV Shows.",
                ));
            }
            SearchStatus::Results => {
                let total = self
                    .search_total_results
                    .unwrap_or(self.search_results.len() as u32);
                page =
                    page.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_lg()
                                    .text_color(rgb(Theme::TEXT_PRIMARY))
                                    .child("Search results"),
                            )
                            .child(div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child(
                                format!("Showing {} of {total}", self.search_results.len()),
                            )),
                    );

                let mut results = div().w_full().flex().flex_wrap().gap_4();
                for (index, card) in self.search_results.iter().enumerate() {
                    let selected = self.selected_search_result == Some(index);
                    let tile = div()
                        .id(format!("search-result-{}-{index}", card.id))
                        .tab_index(40 + index as isize)
                        .rounded_lg()
                        .border_1()
                        .border_color(rgb(if selected {
                            Theme::ACCENT
                        } else {
                            Theme::BORDER
                        }))
                        .p_1()
                        .cursor_pointer()
                        .hover(|style| style.border_color(rgb(Theme::ACCENT_DARK)))
                        .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                        .aria_label(format!("Preview {}", card.title))
                        .on_click(cx.listener(move |this, _event, _window, cx| {
                            this.selected_search_result = Some(index);
                            cx.notify();
                        }))
                        .child(poster_card(
                            &card.title,
                            &card.detail,
                            card.poster.clone(),
                            self.search_artwork_loading,
                        ));
                    results = results.child(tile);
                }
                page = page.child(results);

                if let Some(card) = self
                    .selected_search_result
                    .and_then(|index| self.search_results.get(index))
                {
                    let synopsis = card
                        .overview
                        .as_deref()
                        .filter(|overview| !overview.trim().is_empty())
                        .unwrap_or("No synopsis is available for this title.");
                    let media_label = match card.media_type {
                        MediaType::Movie => "MOVIE",
                        MediaType::Tv => "TV SERIES",
                        MediaType::Person => "PERSON",
                        MediaType::Other => "TITLE",
                    };
                    let mut preview = div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .rounded_lg()
                        .border_1()
                        .border_color(rgb(Theme::BORDER))
                        .bg(rgb(Theme::SURFACE))
                        .p_5()
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(Theme::ACCENT))
                                .child(format!("QUICK PREVIEW · {media_label}")),
                        )
                        .child(
                            div()
                                .text_xl()
                                .text_color(rgb(Theme::TEXT_PRIMARY))
                                .child(card.title.clone()),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(Theme::TEXT_SECONDARY))
                                .child(synopsis.to_owned()),
                        );

                    if let Some(rating) = card.vote_average {
                        preview = preview.child(
                            div()
                                .text_xs()
                                .text_color(rgb(Theme::TEXT_MUTED))
                                .child(format!("TMDB rating · {rating:.1}/10")),
                        );
                    }
                    page = page.child(preview);
                }

                if self.search_artwork_loading {
                    page = page.child(
                        div()
                            .text_xs()
                            .text_color(rgb(Theme::TEXT_MUTED))
                            .child("Loading validated artwork…"),
                    );
                }
            }
        }

        page
    }

    fn render_home_artwork(&self, cx: &mut Context<Self>) -> gpui::Div {
        let heading = div()
            .text_lg()
            .text_color(rgb(Theme::TEXT_PRIMARY))
            .child("Trending this week");

        let content = match self.home_catalogue_status {
            HomeCatalogueStatus::Loading | HomeCatalogueStatus::NotRequested => {
                let mut row = div().flex().gap_4();
                for _ in 0..7 {
                    row = row.child(poster_card(" ", " ", None, true));
                }
                row
            }
            HomeCatalogueStatus::Loaded if self.home_cards.is_empty() => {
                return div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(heading)
                    .child(status_panel(
                        StatusTone::Empty,
                        "NO ARTWORK AVAILABLE",
                        "Nothing to show yet",
                        "The metadata catalogue did not return titles with poster artwork.",
                    ));
            }
            HomeCatalogueStatus::Loaded => {
                let mut row = div().flex().gap_4();
                for card in &self.home_cards {
                    row = row.child(poster_card(
                        &card.title,
                        &card.detail,
                        card.poster.clone(),
                        false,
                    ));
                }
                row
            }
        };

        let mut section = div()
            .w_full()
            .flex()
            .flex_col()
            .gap_3()
            .child(heading)
            .child(
                div()
                    .id("trending-shelf-scroll")
                    .w_full()
                    .overflow_x_scroll()
                    .child(content),
            );

        if let Some(error) = &self.home_catalogue_error {
            section = section.child(status_panel(
                StatusTone::Error,
                "CATALOGUE UNAVAILABLE",
                "Trending titles could not be loaded",
                error,
            ));
            section = section.child(
                div()
                    .id("retry-trending-catalogue")
                    .tab_index(34)
                    .rounded_md()
                    .bg(rgb(Theme::ACCENT_DARK))
                    .px_3()
                    .py_2()
                    .cursor_pointer()
                    .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                    .aria_label("Retry trending artwork")
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        this.load_home_catalogue(cx);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::ACCENT))
                            .child("Retry"),
                    ),
            );
        }

        section
    }

    fn nav_item(&self, page: AppPage, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let active = self.navigation.current() == page;
        let focus = self
            .navigation_focus
            .get(&page)
            .expect("every primary page has a focus handle")
            .clone();

        div()
            .id(format!("navigation-{}", page.key()))
            .track_focus(&focus)
            .tab_index(
                AppPage::ALL
                    .iter()
                    .position(|item| *item == page)
                    .unwrap_or(0) as isize,
            )
            .key_context("SENZO_NAVIGATION")
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .rounded_md()
            .border_1()
            .border_color(if active {
                rgb(Theme::ACCENT_DARK)
            } else {
                rgb(Theme::SIDEBAR)
            })
            .bg(if active {
                rgb(Theme::ACCENT_DARK)
            } else {
                rgb(Theme::SIDEBAR)
            })
            .px_3()
            .py_3()
            .cursor_pointer()
            .hover(|style| style.bg(rgb(Theme::SURFACE_HOVER)))
            .focus_visible(|style| {
                style
                    .border_1()
                    .border_color(rgb(Theme::ACCENT))
                    .bg(rgb(Theme::SURFACE_HOVER))
            })
            .aria_label(format!("{} navigation", page.title()))
            .aria_selected(active)
            .aria_keyshortcuts("Enter, Space")
            .on_click(cx.listener(move |this, _event, window, cx| {
                this.navigate(page, window, cx);
            }))
            .on_action(
                cx.listener(move |this, _: &ActivateNavigation, window, cx| {
                    this.navigate(page, window, cx);
                }),
            )
            .child(
                div()
                    .w(px(38.0))
                    .h(px(38.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .bg(if active {
                        rgb(Theme::ACCENT)
                    } else {
                        rgb(Theme::SURFACE_RAISED)
                    })
                    .text_color(if active {
                        rgb(Theme::BACKGROUND)
                    } else {
                        rgb(Theme::TEXT_SECONDARY)
                    })
                    .text_xs()
                    .child(page.short_label()),
            )
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .text_color(if active {
                        rgb(Theme::TEXT_PRIMARY)
                    } else {
                        rgb(Theme::TEXT_SECONDARY)
                    })
                    .child(page.title()),
            )
            .when(active, |element| {
                element.child(
                    div()
                        .w(px(5.0))
                        .h(px(5.0))
                        .rounded_full()
                        .bg(rgb(Theme::ACCENT)),
                )
            })
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .id("senzo-sidebar")
            .key_context("SENZO_NAVIGATION")
            .w(px(Theme::SIDEBAR_WIDTH))
            .h_full()
            .flex()
            .flex_col()
            .gap_5()
            .bg(rgb(Theme::SIDEBAR))
            .border_r_1()
            .border_color(rgb(Theme::BORDER))
            .p_5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(img(self.logo.clone()).size(px(44.0)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xl()
                                    .text_color(rgb(Theme::TEXT_PRIMARY))
                                    .child("SENZO"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(Theme::TEXT_MUTED))
                                    .child("NATIVE MEDIA CENTER"),
                            ),
                    ),
            )
            .child(div().w_full().h(px(1.0)).bg(rgb(Theme::BORDER)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .px_2()
                            .text_xs()
                            .text_color(rgb(Theme::TEXT_MUTED))
                            .child("DISCOVER"),
                    )
                    .child(self.nav_item(AppPage::Home, cx))
                    .child(self.nav_item(AppPage::Movies, cx))
                    .child(self.nav_item(AppPage::TvShows, cx))
                    .child(self.nav_item(AppPage::Search, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .px_2()
                            .text_xs()
                            .text_color(rgb(Theme::TEXT_MUTED))
                            .child("YOUR SPACE"),
                    )
                    .child(self.nav_item(AppPage::Library, cx))
                    .child(self.nav_item(AppPage::Addons, cx)),
            )
            .child(div().flex_1())
            .child(self.nav_item(AppPage::Settings, cx))
            .child(
                div()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(Theme::BORDER))
                    .bg(rgb(Theme::SURFACE))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(Theme::ACCENT))
                            .child("SENZO DESKTOP"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(Theme::TEXT_MUTED))
                            .child("Rust · GPUI CE · Windows + Linux"),
                    ),
            )
    }

    fn render_header(&self, cx: &mut Context<Self>) -> gpui::Div {
        let page = self.navigation.current();
        div()
            .w_full()
            .h(px(Theme::HEADER_HEIGHT))
            .flex()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(rgb(Theme::BORDER))
            .px_8()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_2xl()
                            .text_color(rgb(Theme::TEXT_PRIMARY))
                            .child(page.title()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(Theme::TEXT_MUTED))
                            .child(page.description()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id("fullscreen-button")
                            .tab_index(20)
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(Theme::BORDER))
                            .bg(rgb(Theme::SURFACE))
                            .px_3()
                            .py_2()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(Theme::SURFACE_HOVER)))
                            .focus_visible(|style| {
                                style.border_1().border_color(rgb(Theme::ACCENT))
                            })
                            .aria_label("Toggle fullscreen")
                            .aria_keyshortcuts("F11")
                            .on_click(|_event, window, _cx| window.toggle_fullscreen())
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(Theme::TEXT_SECONDARY))
                                    .child("Fullscreen"),
                            ),
                    )
                    .child(
                        div()
                            .id("more-menu-button")
                            .tab_index(21)
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(Theme::BORDER))
                            .bg(if self.menu_open {
                                rgb(Theme::ACCENT_DARK)
                            } else {
                                rgb(Theme::SURFACE)
                            })
                            .px_3()
                            .py_2()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(Theme::SURFACE_HOVER)))
                            .focus_visible(|style| {
                                style.border_1().border_color(rgb(Theme::ACCENT))
                            })
                            .aria_label("Open application menu")
                            .aria_expanded(self.menu_open)
                            .on_click(cx.listener(|this, _event, _window, cx| {
                                this.menu_open = !this.menu_open;
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(Theme::TEXT_PRIMARY))
                                    .child("More  ···"),
                            ),
                    ),
            )
    }

    fn render_home(&self, cx: &mut Context<Self>) -> gpui::Div {
        let featured_title = self
            .home_featured_title
            .as_deref()
            .or_else(|| self.home_cards.first().map(|card| card.title.as_str()))
            .unwrap_or("Your entertainment. Your space.");
        let featured_eyebrow = if self.home_catalogue_status == HomeCatalogueStatus::Loaded {
            "TRENDING THIS WEEK"
        } else {
            "NATIVE MEDIA CENTER"
        };
        let mut hero = backdrop_surface(
            self.home_featured_backdrop.clone(),
            286.0,
            self.home_catalogue_status == HomeCatalogueStatus::Loading,
        )
        .child(
            div()
                .absolute()
                .inset_0()
                .bg(rgb(Theme::BACKGROUND).opacity(0.76)),
        );
        hero = hero.child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .justify_end()
                .gap_3()
                .p_6()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(Theme::ACCENT))
                        .child(featured_eyebrow),
                )
                .child(
                    div()
                        .text_2xl()
                        .text_color(rgb(Theme::TEXT_PRIMARY))
                        .child(featured_title.to_owned()),
                )
                .child(
                    div()
                        .max_w(px(720.0))
                        .text_sm()
                        .text_color(rgb(Theme::TEXT_SECONDARY))
                        .child("Real catalogue artwork, validated and decoded by SENZO's native Rust pipeline, rendered directly by GPUI."),
                )
                .child(
                    div()
                        .flex()
                        .gap_3()
                        .child(
                            div()
                                .id("home-explore-movies")
                                .tab_index(30)
                                .rounded_md()
                                .bg(rgb(Theme::ACCENT))
                                .px_4()
                                .py_3()
                                .cursor_pointer()
                                .hover(|style| style.bg(rgb(0x67E8F9)))
                                .focus_visible(|style| style.border_1().border_color(rgb(Theme::TEXT_PRIMARY)))
                                .aria_label("Browse movies")
                                .on_click(cx.listener(|this, _event, window, cx| {
                                    this.navigate(AppPage::Movies, window, cx);
                                }))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(Theme::BACKGROUND))
                                        .child("Explore movies"),
                                ),
                        )
                        .child(
                            div()
                                .id("home-open-addons")
                                .tab_index(31)
                                .rounded_md()
                                .border_1()
                                .border_color(rgb(Theme::BORDER_STRONG))
                                .bg(rgb(Theme::SURFACE_RAISED))
                                .px_4()
                                .py_3()
                                .cursor_pointer()
                                .hover(|style| style.bg(rgb(Theme::SURFACE_HOVER)))
                                .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                                .aria_label("Open add-ons")
                                .on_click(cx.listener(|this, _event, window, cx| {
                                    this.navigate(AppPage::Addons, window, cx);
                                }))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(Theme::TEXT_PRIMARY))
                                        .child("Manage add-ons"),
                                ),
                        ),
                ),
        );

        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_6()
            .child(hero)
            .child(self.render_home_artwork(cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_lg()
                            .text_color(rgb(Theme::TEXT_PRIMARY))
                            .child("Built for your screen"),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(feature_tile(
                                "DISCOVER",
                                "Movies & TV",
                                "A unified place for metadata, catalogues and recommendations.",
                            ))
                            .child(feature_tile(
                                "ORGANIZE",
                                "Your library",
                                "Keep saved titles, collections and viewing progress together.",
                            ))
                            .child(feature_tile(
                                "EXTEND",
                                "Your providers",
                                "Choose and manage compatible add-ons when you are ready.",
                            )),
                    ),
            )
    }

    fn render_settings(&self, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_5()
            .child(page_intro(
                "PREFERENCES",
                "Settings",
                "Everyday controls will stay simple, with advanced options kept separate.",
            ))
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(feature_tile(
                        "APPEARANCE",
                        "Display & appearance",
                        "Theme, window behavior and visual preferences.",
                    ))
                    .child(feature_tile(
                        "PLAYBACK",
                        "Playback",
                        "Player behavior, audio and subtitle preferences.",
                    ))
                    .child(feature_tile(
                        "LANGUAGE",
                        "Language",
                        "App language and preferred media languages.",
                    )),
            )
            .child(status_panel(
                if self.runtime_status.contains("failed") {
                    StatusTone::Error
                } else {
                    StatusTone::Info
                },
                "ADVANCED DIAGNOSTICS",
                "Background runtime",
                "A lightweight check confirms that the application-owned Tokio runtime can execute background work without blocking the GPUI interface.",
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(Theme::BORDER))
                    .bg(rgb(Theme::SURFACE))
                    .p_4()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child(self.runtime_status.clone()),
                    )
                    .child(
                        div()
                            .id("runtime-check")
                            .tab_index(40)
                            .rounded_md()
                            .bg(rgb(Theme::ACCENT_DARK))
                            .px_3()
                            .py_2()
                            .cursor_pointer()
                            .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                            .aria_label("Run background runtime check")
                            .on_click(cx.listener(|this, _event, _window, cx| {
                                this.check_runtime(cx);
                            }))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(Theme::ACCENT))
                                    .child("Run check"),
                            ),
                    ),
            )
    }

    fn render_page(&self, cx: &mut Context<Self>) -> gpui::Div {
        match self.navigation.current() {
            AppPage::Home => self.render_home(cx),
            AppPage::Movies => div()
                .w_full()
                .flex()
                .flex_col()
                .gap_5()
                .child(page_intro(
                    "DISCOVER",
                    "Movies",
                    "Find films through trusted metadata and the catalogues you choose.",
                ))
                .child(status_panel(
                    StatusTone::Loading,
                    "COMING IN PHASE 3",
                    "Movie discovery is being connected",
                    "The native metadata client, catalogue requests and artwork pipeline are the next implementation milestone. No sample or invented titles are shown here.",
                )),
            AppPage::TvShows => div()
                .w_full()
                .flex()
                .flex_col()
                .gap_5()
                .child(page_intro(
                    "DISCOVER",
                    "TV Shows",
                    "Explore series, seasons and episodes in one native experience.",
                ))
                .child(status_panel(
                    StatusTone::Loading,
                    "COMING IN PHASE 3",
                    "TV discovery is being connected",
                    "Series metadata, season and episode details will populate this route when the native API client is implemented.",
                )),
            AppPage::Search => self.render_search(cx),
            AppPage::Library => div()
                .w_full()
                .flex()
                .flex_col()
                .gap_5()
                .child(page_intro(
                    "YOUR SPACE",
                    "Library",
                    "Your saved titles and personal collections belong here.",
                ))
                .child(status_panel(
                    StatusTone::Empty,
                    "EMPTY LIBRARY",
                    "Your library is empty",
                    "When library and collection workflows are implemented, saved media will appear here. Your local data foundation is already in place.",
                )),
            AppPage::Addons => div()
                .w_full()
                .flex()
                .flex_col()
                .gap_5()
                .child(page_intro(
                    "EXTEND SENZO",
                    "Add-ons",
                    "Choose which external catalogues and stream providers SENZO can use.",
                ))
                .child(status_panel(
                    StatusTone::Empty,
                    "NO INSTALLED ADD-ONS",
                    "Nothing has been installed",
                    "Stremio-compatible add-on support arrives in Phase 6. Add-ons will require explicit user installation; Torrentio will never be bundled or enabled by default.",
                )),
            AppPage::Settings => self.render_settings(cx),
        }
    }

    fn render_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .id("application-menu")
            .absolute()
            .top(px(Theme::HEADER_HEIGHT - 4.0))
            .right(px(28.0))
            .w(px(248.0))
            .flex()
            .flex_col()
            .gap_1()
            .rounded_lg()
            .border_1()
            .border_color(rgb(Theme::BORDER_STRONG))
            .bg(rgb(Theme::SURFACE_RAISED))
            .p_2()
            .aria_label("Application menu")
            .child(self.menu_item("Keyboard shortcuts", 50, cx.listener(|this, _event, window, cx| {
                this.open_dialog(DialogKind::Shortcuts, window, cx);
            })))
            .child(self.menu_item("About SENZO", 51, cx.listener(|this, _event, window, cx| {
                this.open_dialog(DialogKind::About, window, cx);
            })))
            .child(self.menu_item("What's new", 52, cx.listener(|this, _event, _window, cx| {
                this.menu_open = false;
                this.notification = Some("Phase 2 brings SENZO's native navigation, design system and desktop shell.".to_owned());
                cx.notify();
            })))
            .child(div().w_full().h(px(1.0)).bg(rgb(Theme::BORDER)))
            .child(self.menu_item("Quit SENZO", 53, cx.listener(|_, _event, _window, cx| cx.quit())))
    }

    fn menu_item<F>(
        &self,
        label: &'static str,
        tab_index: isize,
        on_click: F,
    ) -> impl IntoElement + use<F>
    where
        F: Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    {
        div()
            .id(format!("menu-item-{label}"))
            .tab_index(tab_index)
            .w_full()
            .rounded_md()
            .px_3()
            .py_3()
            .cursor_pointer()
            .hover(|style| style.bg(rgb(Theme::SURFACE_HOVER)))
            .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
            .aria_label(label)
            .aria_keyshortcuts("Enter")
            .on_click(on_click)
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(Theme::TEXT_PRIMARY))
                    .child(label),
            )
    }

    fn render_dialog(&self, kind: DialogKind, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (eyebrow, title, description) = match kind {
            DialogKind::About => (
                "ABOUT",
                "SENZO",
                "A native desktop media center, re-engineered in Rust with GPUI CE.",
            ),
            DialogKind::Shortcuts => (
                "KEYBOARD",
                "Keyboard shortcuts",
                "Navigate SENZO without reaching for the mouse.",
            ),
        };

        let dialog = div()
            .id("dialog-overlay")
            .aria_label("SENZO dialog")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(0x000000).opacity(0.78))
            .child(
                div()
                    .w(px(540.0))
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(Theme::BORDER_STRONG))
                    .bg(rgb(Theme::SURFACE_RAISED))
                    .p_6()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(Theme::ACCENT))
                            .child(eyebrow),
                    )
                    .child(
                        div()
                            .text_2xl()
                            .text_color(rgb(Theme::TEXT_PRIMARY))
                            .child(title),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child(description),
                    ),
            );

        let details = match kind {
            DialogKind::About => vec![
                ("Version", "0.1.0 · development"),
                ("Technology", "Rust 2024 · GPUI CE 0.2.2"),
                ("Platforms", "Native Windows and Linux"),
                ("Playback", "MPV/libmpv integration is planned"),
            ],
            DialogKind::Shortcuts => vec![
                ("Back", "Escape or Alt + Left"),
                ("Forward", "Alt + Right"),
                ("Fullscreen", "F11"),
                ("Sidebar", "Up / Down, then Enter or Space"),
                ("Focus", "Tab / Shift + Tab"),
                ("Quit", "Ctrl + Q or Cmd + Q"),
            ],
        };

        let rows =
            details
                .into_iter()
                .fold(div().flex().flex_col().gap_3(), |column, (label, value)| {
                    column.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_4()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(Theme::TEXT_SECONDARY))
                                    .child(label),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(Theme::TEXT_PRIMARY))
                                    .child(value),
                            ),
                    )
                });

        dialog.child(rows).child(divider()).child(
            div().flex().justify_end().child(
                div()
                    .id("dialog-close")
                    .track_focus(&self.dialog_focus)
                    .tab_index(60)
                    .rounded_md()
                    .bg(rgb(Theme::ACCENT))
                    .px_4()
                    .py_2()
                    .cursor_pointer()
                    .focus_visible(|style| style.border_1().border_color(rgb(Theme::TEXT_PRIMARY)))
                    .aria_label("Close dialog")
                    .on_click(cx.listener(|this, _event, window, cx| {
                        this.dialog = None;
                        this.focus_page(this.navigation.current(), window, cx);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::BACKGROUND))
                            .child("Close"),
                    ),
            ),
        )
    }

    fn render_notification(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let message = self.notification.clone().unwrap_or_default();
        div()
            .id("notification-toast")
            .absolute()
            .bottom(px(24.0))
            .right(px(24.0))
            .max_w(px(420.0))
            .flex()
            .items_center()
            .gap_3()
            .rounded_md()
            .border_1()
            .border_color(rgb(Theme::BORDER_STRONG))
            .bg(rgb(Theme::SURFACE_RAISED))
            .p_4()
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .text_color(rgb(Theme::TEXT_PRIMARY))
                    .child(message),
            )
            .child(
                div()
                    .id("dismiss-notification")
                    .tab_index(70)
                    .rounded_md()
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(Theme::SURFACE_HOVER)))
                    .aria_label("Dismiss notification")
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        this.notification = None;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child("Dismiss"),
                    ),
            )
    }
}

async fn load_gpui_artwork(
    artwork: ArtworkService,
    file_path: String,
    size: ArtworkSize,
    cancellation: CancellationToken,
) -> Option<Arc<RenderImage>> {
    let url = ArtworkUrlResolver
        .resolve_tmdb_path(&file_path, size)
        .ok()?;
    let decoded = artwork.load(&url, &cancellation).await.ok()?;
    tokio::task::spawn_blocking(move || {
        render_image_from_artwork(decoded.width(), decoded.height(), decoded.rgba8())
    })
    .await
    .ok()?
    .ok()
}

impl Drop for SenzoShell {
    fn drop(&mut self) {
        self.home_cancellation.cancel();
        self.search_cancellation.cancel();
    }
}

impl Render for SenzoShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.smoke_test
            && self.navigation.current() == AppPage::Home
            && self.home_catalogue_status == HomeCatalogueStatus::NotRequested
            && self.home_catalogue_error.is_none()
        {
            self.load_home_catalogue(cx);
        }

        if self.smoke_test && !self.smoke_test_scheduled {
            self.smoke_test_scheduled = true;
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(900))
                    .await;
                let _ = this.update(cx, |_, cx| {
                    tracing::info!("native SENZO shell opened and rendered; ending smoke test");
                    cx.quit();
                });
            })
            .detach();
        }

        let sidebar = self.render_sidebar(cx);
        let header = self.render_header(cx);
        let page = self.render_page(cx);

        let mut root = div()
            .id("senzo-root")
            .key_context("SENZO_ROOT")
            .relative()
            .size_full()
            .flex()
            .bg(rgb(Theme::BACKGROUND))
            .text_color(rgb(Theme::TEXT_PRIMARY))
            .on_action(cx.listener(|this, _: &GoBack, window, cx| {
                this.go_back(window, cx);
            }))
            .on_action(cx.listener(|this, _: &GoForward, window, cx| {
                this.go_forward(window, cx);
            }))
            .on_action(|_: &ToggleFullscreen, window, _cx| {
                window.toggle_fullscreen();
            })
            .on_action(|_: &FocusNext, window, cx| {
                window.focus_next(cx);
            })
            .on_action(|_: &FocusPrevious, window, cx| {
                window.focus_prev(cx);
            })
            .on_action(cx.listener(|this, _: &MoveNavigationUp, window, cx| {
                this.move_nav_focus(-1, window, cx);
            }))
            .on_action(cx.listener(|this, _: &MoveNavigationDown, window, cx| {
                this.move_nav_focus(1, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowShortcuts, window, cx| {
                this.open_dialog(DialogKind::Shortcuts, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowAbout, window, cx| {
                this.open_dialog(DialogKind::About, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowWhatsNew, _window, cx| {
                this.menu_open = false;
                this.notification = Some(
                    "Phase 2 brings SENZO's native navigation, design system and desktop shell."
                        .to_owned(),
                );
                cx.notify();
            }))
            .on_action(|_: &Quit, _window, cx| cx.quit())
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Back),
                cx.listener(|this, _event, window, cx| this.go_back(window, cx)),
            )
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Forward),
                cx.listener(|this, _event, window, cx| this.go_forward(window, cx)),
            )
            .child(sidebar)
            .child(
                div()
                    .h_full()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(header)
                    .child(
                        div()
                            .id("page-content")
                            .flex_1()
                            .overflow_y_scroll()
                            .p_8()
                            .child(page),
                    ),
            );

        if self.menu_open {
            root = root.child(self.render_menu(cx));
        }
        if let Some(kind) = self.dialog {
            root = root.child(self.render_dialog(kind, cx));
        }
        if self.notification.is_some() {
            root = root.child(self.render_notification(cx));
        }

        root
    }
}

fn bind_shell_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", GoBack, Some("SENZO_ROOT")),
        KeyBinding::new("alt-left", GoBack, Some("SENZO_ROOT")),
        KeyBinding::new("alt-right", GoForward, Some("SENZO_ROOT")),
        KeyBinding::new("f11", ToggleFullscreen, Some("SENZO_ROOT")),
        KeyBinding::new("alt-enter", ToggleFullscreen, Some("SENZO_ROOT")),
        KeyBinding::new("tab", FocusNext, Some("SENZO_ROOT")),
        KeyBinding::new("shift-tab", FocusPrevious, Some("SENZO_ROOT")),
        KeyBinding::new("up", MoveNavigationUp, Some("SENZO_NAVIGATION")),
        KeyBinding::new("down", MoveNavigationDown, Some("SENZO_NAVIGATION")),
        KeyBinding::new("enter", ActivateNavigation, Some("SENZO_NAVIGATION")),
        KeyBinding::new("space", ActivateNavigation, Some("SENZO_NAVIGATION")),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("ctrl-q", Quit, None),
        KeyBinding::new("cmd-shift-/", ShowShortcuts, None),
    ]);
}

pub fn run(tokio: Handle, smoke_test: bool, metadata: MetadataClient, artwork: ArtworkService) {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            bind_shell_keys(cx);

            cx.set_menus(vec![
                Menu {
                    name: "SENZO".into(),
                    disabled: false,
                    items: vec![
                        MenuItem::action("About SENZO", ShowAbout),
                        MenuItem::action("Keyboard shortcuts", ShowShortcuts),
                        MenuItem::action("Quit SENZO", Quit),
                    ],
                },
                Menu {
                    name: "Navigate".into(),
                    disabled: false,
                    items: vec![
                        MenuItem::action("Back", GoBack),
                        MenuItem::action("Forward", GoForward),
                    ],
                },
                Menu {
                    name: "View".into(),
                    disabled: false,
                    items: vec![MenuItem::action("Toggle Fullscreen", ToggleFullscreen)],
                },
            ]);

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1280.0), px(720.0)),
                    cx,
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::from("SENZO")),
                    ..Default::default()
                }),
                app_id: Some("com.senzo.desktop".to_owned()),
                window_min_size: Some(size(
                    px(Theme::MIN_WINDOW_WIDTH),
                    px(Theme::MIN_WINDOW_HEIGHT),
                )),
                ..Default::default()
            };

            cx.open_window(options, move |window, cx| {
                let shell =
                    cx.new(|cx| SenzoShell::new(tokio, smoke_test, metadata, artwork, window, cx));
                let initial_focus = shell
                    .read(cx)
                    .navigation_focus
                    .get(&AppPage::Home)
                    .expect("home navigation focus is initialized")
                    .clone();
                window.focus(&initial_focus, cx);
                shell
            })
            .expect("failed to open the SENZO native window");
            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_filters_map_to_the_hosted_metadata_contract() {
        assert_eq!(SearchFilter::All.search_type(), SearchType::Multi);
        assert_eq!(SearchFilter::Movies.search_type(), SearchType::Movie);
        assert_eq!(SearchFilter::TvShows.search_type(), SearchType::Tv);
    }

    #[gpui::test]
    async fn keyboard_navigation_back_forward_and_focus_restoration(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test Tokio runtime should initialize");

        let http = senzo_network::HttpClient::new(Default::default())
            .expect("test HTTP client should initialize");
        let metadata = MetadataClient::new(http.clone(), Default::default())
            .expect("test metadata client should initialize");
        let artwork = ArtworkService::new(
            http,
            std::env::temp_dir().join("senzo-ui-artwork-test-cache"),
            Default::default(),
        )
        .expect("test artwork cache should initialize");

        let (shell, window_cx) = cx.add_window_view(|window, cx| {
            let mut shell = SenzoShell::new(
                runtime.handle().clone(),
                false,
                metadata,
                artwork,
                window,
                cx,
            );
            shell.home_catalogue_status = HomeCatalogueStatus::Loaded;
            let home_focus = shell
                .navigation_focus
                .get(&AppPage::Home)
                .expect("home focus handle exists")
                .clone();
            window.focus(&home_focus, cx);
            shell
        });

        window_cx.update(|_, app| bind_shell_keys(app));

        for expected in [
            AppPage::Movies,
            AppPage::TvShows,
            AppPage::Search,
            AppPage::Library,
            AppPage::Addons,
            AppPage::Settings,
        ] {
            window_cx.simulate_keystrokes("down enter");
            assert_eq!(
                shell.read_with(window_cx, |shell, _| shell.navigation.current()),
                expected
            );
        }

        window_cx.simulate_keystrokes("down enter");
        assert_eq!(
            shell.read_with(window_cx, |shell, _| shell.navigation.current()),
            AppPage::Home
        );
        window_cx.simulate_keystrokes("escape");
        assert_eq!(
            shell.read_with(window_cx, |shell, _| shell.navigation.current()),
            AppPage::Settings
        );
        window_cx.simulate_keystrokes("alt-left");
        assert_eq!(
            shell.read_with(window_cx, |shell, _| shell.navigation.current()),
            AppPage::Addons
        );
        window_cx.simulate_keystrokes("alt-right");
        assert_eq!(
            shell.read_with(window_cx, |shell, _| shell.navigation.current()),
            AppPage::Settings
        );
        assert!(window_cx.update(|window, app| {
            shell
                .read(app)
                .navigation_focus
                .get(&AppPage::Settings)
                .expect("settings focus handle exists")
                .is_focused(window)
        }));

        window_cx.dispatch_action(ShowShortcuts);
        assert_eq!(
            shell.read_with(window_cx, |shell, _| shell.dialog),
            Some(DialogKind::Shortcuts)
        );
        window_cx.simulate_keystrokes("escape");
        assert_eq!(shell.read_with(window_cx, |shell, _| shell.dialog), None);
        assert!(window_cx.update(|window, app| {
            shell
                .read(app)
                .navigation_focus
                .get(&AppPage::Settings)
                .expect("settings focus handle exists")
                .is_focused(window)
        }));
    }
}
