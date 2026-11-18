//! Native SENZO desktop shell: routes, focus, menus, dialogs and window actions.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use gpui::{
    App, Bounds, ColorExt, Context, Entity, FocusHandle, Image, ImageFormat, KeyBinding, Menu,
    MenuItem, MouseButton, NavigationDirection, QuitMode, Render, RenderImage, SharedString,
    Subscription, TitlebarOptions, Window, WindowBounds, WindowOptions, actions, div, img,
    prelude::*, px, rgb, size,
};
use gpui_component::input::{Input, InputEvent, InputState};
use senzo_artwork::{ArtworkService, ArtworkSize, ArtworkUrlResolver};
use senzo_catalog::HomeSettings;
use senzo_domain::{AppPage, SettingsRepository};
#[cfg(test)]
use senzo_metadata::PersonCredit;
use senzo_metadata::{
    MediaItem, MediaType, MetadataClient, MetadataError, Person, SearchType, SeasonDetails,
    SeasonSummary,
};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum HomeShelfKey {
    Trending,
    PopularMovies,
    PopularTv,
}

impl HomeShelfKey {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Trending => "trending",
            Self::PopularMovies => "popular-movies",
            Self::PopularTv => "popular-tv",
        }
    }
}

const MAX_HOME_FEATURED_ITEMS: usize = 8;
const MAX_HOME_SHELF_ITEMS: usize = 120;
const MAX_HOME_CATALOGUE_PAGES: u32 = 500;

#[derive(Clone)]
struct HomeMediaCandidate {
    id: u64,
    media_type: MediaType,
    title: String,
    detail: String,
    overview: Option<String>,
    poster_path: String,
    backdrop_path: Option<String>,
}

struct HomeFeaturedCard {
    id: u64,
    media_type: MediaType,
    title: String,
    detail: String,
    overview: Option<String>,
    poster: Option<Arc<RenderImage>>,
    backdrop: Arc<RenderImage>,
}

fn selected_home_hero_sources(settings: &HomeSettings) -> Vec<HomeShelfKey> {
    settings
        .sections
        .iter()
        .filter(|section| section.enabled && section.hero_source_enabled)
        .filter_map(|section| match section.key.as_str() {
            "trending" => Some(HomeShelfKey::Trending),
            "popular-movies" => Some(HomeShelfKey::PopularMovies),
            "popular-tv" => Some(HomeShelfKey::PopularTv),
            _ => None,
        })
        .take(2)
        .collect()
}

fn next_home_featured_index(current: usize, item_count: usize, direction: isize) -> usize {
    if item_count == 0 {
        return 0;
    }
    (current as isize + direction.signum()).rem_euclid(item_count as isize) as usize
}

struct HomeMediaCard {
    id: u64,
    media_type: MediaType,
    title: String,
    detail: String,
    poster: Option<Arc<RenderImage>>,
}

struct HomeShelfState {
    status: HomeCatalogueStatus,
    error: Option<String>,
    cards: Vec<HomeMediaCard>,
    page: u32,
    total_pages: u32,
    loading_more: bool,
    load_more_error: Option<String>,
}

impl Default for HomeShelfState {
    fn default() -> Self {
        Self {
            status: HomeCatalogueStatus::NotRequested,
            error: None,
            cards: Vec::new(),
            page: 0,
            total_pages: 1,
            loading_more: false,
            load_more_error: None,
        }
    }
}

impl HomeShelfState {
    fn can_load_more(&self) -> bool {
        self.status == HomeCatalogueStatus::Loaded
            && self.page > 0
            && self.page < self.total_pages
            && self.cards.len() < MAX_HOME_SHELF_ITEMS
    }
}

struct HomeShelfLoadData {
    candidates: Vec<HomeMediaCandidate>,
    cards: Vec<HomeMediaCard>,
    error: Option<String>,
    page: u32,
    total_pages: u32,
}

struct HomeShelfPageData {
    page: u32,
    total_pages: u32,
    cards: Vec<HomeMediaCard>,
}

struct HomeCatalogueData {
    trending: HomeShelfLoadData,
    popular_movies: HomeShelfLoadData,
    popular_tv: HomeShelfLoadData,
    featured: Vec<HomeFeaturedCard>,
}

fn home_media_candidates(
    items: Vec<MediaItem>,
    fallback_type: Option<MediaType>,
) -> Vec<HomeMediaCandidate> {
    items
        .into_iter()
        .filter_map(|media| {
            let media_type = media.media_type.or(fallback_type)?;
            let poster_path = match media_type {
                MediaType::Movie | MediaType::Tv => media.poster_path.clone()?,
                MediaType::Person => media.profile_path.clone()?,
                MediaType::Other => return None,
            };
            let title = media.display_title().trim();
            if title.is_empty() || title == "Untitled" {
                return None;
            }
            let category = match media_type {
                MediaType::Movie => "MOVIE",
                MediaType::Tv => "SERIES",
                MediaType::Person => "PERSON",
                MediaType::Other => return None,
            };
            let detail = media
                .release_year()
                .map(|year| format!("{category} · {year}"))
                .unwrap_or_else(|| category.to_owned());
            Some(HomeMediaCandidate {
                id: media.id,
                media_type,
                title: title.to_owned(),
                detail,
                overview: media.overview,
                poster_path,
                backdrop_path: media.backdrop_path,
            })
        })
        .take(12)
        .collect()
}

fn home_shelf_load_data(
    candidates: Vec<HomeMediaCandidate>,
    error: Option<String>,
    page: u32,
    total_pages: Option<u32>,
) -> HomeShelfLoadData {
    let cards = candidates
        .iter()
        .map(|candidate| HomeMediaCard {
            id: candidate.id,
            media_type: candidate.media_type,
            title: candidate.title.clone(),
            detail: candidate.detail.clone(),
            poster: None,
        })
        .collect();
    HomeShelfLoadData {
        candidates,
        cards,
        error,
        page,
        total_pages: bounded_home_total_pages(page, total_pages),
    }
}

fn bounded_home_total_pages(page: u32, total_pages: Option<u32>) -> u32 {
    total_pages
        .unwrap_or(page)
        .max(page)
        .clamp(1, MAX_HOME_CATALOGUE_PAGES)
}

fn home_media_type_key(media_type: MediaType) -> &'static str {
    match media_type {
        MediaType::Movie => "movie",
        MediaType::Tv => "tv",
        MediaType::Person => "person",
        MediaType::Other => "other",
    }
}

fn append_unique_home_cards(
    existing: &mut Vec<HomeMediaCard>,
    incoming: Vec<HomeMediaCard>,
) -> usize {
    let mut identities = existing
        .iter()
        .map(|card| (card.id, home_media_type_key(card.media_type)))
        .collect::<HashSet<_>>();
    let mut added = 0;

    for card in incoming {
        if existing.len() >= MAX_HOME_SHELF_ITEMS {
            break;
        }
        if identities.insert((card.id, home_media_type_key(card.media_type))) {
            existing.push(card);
            added += 1;
        }
    }
    added
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

struct PersonCreditCandidate {
    id: u64,
    media_type: MediaType,
    title: String,
    detail: String,
    popularity: f64,
    artwork_path: String,
    artwork_size: ArtworkSize,
}

struct PersonCreditCard {
    id: u64,
    media_type: MediaType,
    title: String,
    detail: String,
    poster: Option<Arc<RenderImage>>,
}

struct PersonDetailsPage {
    id: u64,
    name: String,
    biography: Option<String>,
    birthday: Option<String>,
    deathday: Option<String>,
    place_of_birth: Option<String>,
    known_for_department: Option<String>,
    profile: Option<Arc<RenderImage>>,
    movie_credits: Vec<PersonCreditCard>,
    tv_credits: Vec<PersonCreditCard>,
    loading: bool,
    error: Option<String>,
}

struct DetailsPage {
    id: u64,
    media_type: MediaType,
    title: String,
    overview: Option<String>,
    tagline: Option<String>,
    release_date: Option<String>,
    runtime: Option<u32>,
    status: Option<String>,
    rating: Option<f64>,
    genres: Vec<String>,
    cast: Vec<(String, String)>,
    seasons: Vec<SeasonSummary>,
    selected_season: Option<SeasonDetails>,
    selected_season_number: Option<u32>,
    season_loading: bool,
    season_error: Option<String>,
    season_cancellation: CancellationToken,
    poster: Option<Arc<RenderImage>>,
    backdrop: Option<Arc<RenderImage>>,
    loading: bool,
    error: Option<String>,
}

pub(crate) struct SenzoShell {
    navigation: NavigationStack,
    navigation_focus: HashMap<AppPage, FocusHandle>,
    dialog_focus: FocusHandle,
    tokio: Handle,
    metadata: MetadataClient,
    artwork: ArtworkService,
    settings: Arc<dyn SettingsRepository>,
    home_settings: HomeSettings,
    home_cancellation: CancellationToken,
    home_catalogue_generation: u64,
    home_catalogue_status: HomeCatalogueStatus,
    home_trending: HomeShelfState,
    home_popular_movies: HomeShelfState,
    home_popular_tv: HomeShelfState,
    home_featured_items: Vec<HomeFeaturedCard>,
    home_featured_index: usize,
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
    details: Option<DetailsPage>,
    details_cancellation: CancellationToken,
    person_details: Option<PersonDetailsPage>,
    person_details_cancellation: CancellationToken,
    logo: Arc<Image>,
    dialog: Option<DialogKind>,
    menu_open: bool,
    notification: Option<String>,
    runtime_status: String,
    smoke_test: bool,
    smoke_test_scheduled: bool,
}

fn metadata_error_message(error: &MetadataError) -> String {
    match error {
        MetadataError::InvalidBaseUrl => {
            "SENZO cannot reach the configured metadata service address. Check SENZO_API_BASE_URL."
                .to_owned()
        }
        MetadataError::InvalidInput(_) => {
            "SENZO could not send this request because its input is invalid. Review the selected title or search query."
                .to_owned()
        }
        MetadataError::Cancelled => "This request was cancelled.".to_owned(),
        MetadataError::ResponseTooLarge => {
            "The metadata service returned a response that exceeded SENZO's safety limit. Try again later."
                .to_owned()
        }
        MetadataError::Api { status: 401 | 403, .. } => {
            "The metadata service rejected the request. Its server-side configuration may need attention."
                .to_owned()
        }
        MetadataError::Api { status: 404, .. } => {
            "That title or catalogue entry could not be found. It may have been removed or may no longer be available."
                .to_owned()
        }
        MetadataError::Api { status: 429, .. } => {
            "The metadata service is receiving too many requests. Wait a moment, then retry."
                .to_owned()
        }
        MetadataError::Api { status, .. } if (500..=599).contains(status) => {
            "The metadata provider is temporarily unavailable. Check your connection and retry shortly."
                .to_owned()
        }
        MetadataError::Api { status, .. } => {
            format!("The metadata service returned HTTP {status}. Retry the request.")
        }
        MetadataError::Network(_) => {
            "SENZO could not connect to its metadata service. Check your internet connection, then retry."
                .to_owned()
        }
        MetadataError::Decode(_) => {
            "The metadata service returned data SENZO could not read. Retry later."
                .to_owned()
        }
        MetadataError::CachePoisoned => {
            "SENZO's metadata cache is unavailable. Restart the app, then retry."
                .to_owned()
        }
    }
}

fn person_credit_candidates(person: &Person, media_type: MediaType) -> Vec<PersonCreditCandidate> {
    let Some(credits) = person.combined_credits.as_ref() else {
        return Vec::new();
    };
    let department = person
        .known_for_department
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let prefer_crew = !department.is_empty() && !matches!(department.as_str(), "acting" | "actors");

    let mut cast = Vec::new();
    let mut crew = Vec::new();
    for (source, destination, is_crew) in [
        (credits.cast.as_slice(), &mut cast, false),
        (credits.crew.as_slice(), &mut crew, true),
    ] {
        for credit in source {
            if credit.media_type != Some(media_type) {
                continue;
            }
            let (artwork_path, artwork_size) = match (
                credit.poster_path.as_deref(),
                credit.backdrop_path.as_deref(),
            ) {
                (Some(path), _) => (path.to_owned(), ArtworkSize::W342),
                (None, Some(path)) => (path.to_owned(), ArtworkSize::W780),
                (None, None) => continue,
            };
            let title = credit.display_title().trim();
            if title.is_empty() || title == "Untitled" {
                continue;
            }
            let media_label = match media_type {
                MediaType::Movie => "MOVIE",
                MediaType::Tv => "TV SERIES",
                MediaType::Person | MediaType::Other => continue,
            };
            let role = if is_crew {
                credit.job.as_deref().or(credit.department.as_deref())
            } else {
                credit.character.as_deref().or(credit.job.as_deref())
            };
            let mut details = vec![media_label.to_owned()];
            if let Some(year) = credit.release_year() {
                details.push(year.to_owned());
            }
            if let Some(role) = role.map(str::trim).filter(|role| !role.is_empty()) {
                details.push(role.to_owned());
            }
            destination.push(PersonCreditCandidate {
                id: credit.id,
                media_type,
                title: title.to_owned(),
                detail: details.join(" · "),
                popularity: credit.popularity.unwrap_or_default(),
                artwork_path,
                artwork_size,
            });
        }
    }

    let mut selected = if prefer_crew && !crew.is_empty() {
        crew
    } else if !cast.is_empty() {
        cast
    } else {
        crew
    };
    selected.sort_by(|left, right| {
        right
            .popularity
            .total_cmp(&left.popularity)
            .then_with(|| left.title.cmp(&right.title))
    });
    let mut seen = HashSet::new();
    selected.retain(|credit| seen.insert(credit.id));
    selected.truncate(20);
    selected
}

impl SenzoShell {
    pub(crate) fn new(
        tokio: Handle,
        smoke_test: bool,
        metadata: MetadataClient,
        artwork: ArtworkService,
        settings: Arc<dyn SettingsRepository>,
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
        let home_settings = HomeSettings::load(settings.as_ref()).unwrap_or_else(|error| {
            tracing::warn!(error = %error, "could not load persisted Home layout; using defaults");
            HomeSettings::default()
        });

        Self {
            navigation: NavigationStack::new(),
            navigation_focus,
            dialog_focus,
            tokio,
            metadata,
            artwork,
            settings,
            home_settings,
            home_cancellation: CancellationToken::new(),
            home_catalogue_generation: 0,
            home_catalogue_status: HomeCatalogueStatus::NotRequested,
            home_trending: HomeShelfState::default(),
            home_popular_movies: HomeShelfState::default(),
            home_popular_tv: HomeShelfState::default(),
            home_featured_items: Vec::new(),
            home_featured_index: 0,
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
            details: None,
            details_cancellation: CancellationToken::new(),
            person_details: None,
            person_details_cancellation: CancellationToken::new(),
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
        if self.details.is_some() {
            self.close_details(cx);
            return;
        }
        if self.person_details.is_some() {
            self.close_person_details(cx);
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

    fn persist_home_settings(&mut self, cx: &mut Context<Self>) {
        match self.home_settings.save(self.settings.as_ref()) {
            Ok(()) => self.notification = Some("Home layout saved locally.".to_owned()),
            Err(error) => {
                tracing::error!(error = %error, "could not persist Home layout");
                self.notification = Some("Could not save the Home layout.".to_owned());
            }
        }
        cx.notify();
    }

    fn toggle_home_hero(&mut self, cx: &mut Context<Self>) {
        self.home_settings
            .set_hero_enabled(!self.home_settings.hero_enabled);
        self.persist_home_settings(cx);
    }

    fn toggle_home_section(&mut self, key: &str, cx: &mut Context<Self>) {
        let enabled = self
            .home_settings
            .sections
            .iter()
            .find(|section| section.key == key)
            .is_some_and(|section| section.enabled);
        let was_hero_source = self
            .home_settings
            .sections
            .iter()
            .find(|section| section.key == key)
            .is_some_and(|section| section.hero_source_enabled);
        if self.home_settings.set_section_enabled(key, !enabled) {
            self.persist_home_settings(cx);
            if was_hero_source {
                self.refresh_home_catalogue(cx);
            }
        }
    }

    fn toggle_home_hero_source(&mut self, key: &str, cx: &mut Context<Self>) {
        let enabled = self
            .home_settings
            .sections
            .iter()
            .find(|section| section.key == key)
            .is_some_and(|section| section.hero_source_enabled);
        if self.home_settings.set_hero_source_enabled(key, !enabled) {
            self.persist_home_settings(cx);
            self.refresh_home_catalogue(cx);
        } else {
            self.notification =
                Some("Home supports up to two enabled hero catalogue sources.".to_owned());
            cx.notify();
        }
    }

    fn refresh_home_catalogue(&mut self, cx: &mut Context<Self>) {
        self.home_cancellation.cancel();
        self.home_cancellation = CancellationToken::new();
        self.home_catalogue_status = HomeCatalogueStatus::NotRequested;
        self.home_trending = HomeShelfState::default();
        self.home_popular_movies = HomeShelfState::default();
        self.home_popular_tv = HomeShelfState::default();
        self.home_featured_items.clear();
        self.home_featured_index = 0;
        self.load_home_catalogue(cx);
    }

    fn home_shelf_mut(&mut self, key: HomeShelfKey) -> &mut HomeShelfState {
        match key {
            HomeShelfKey::Trending => &mut self.home_trending,
            HomeShelfKey::PopularMovies => &mut self.home_popular_movies,
            HomeShelfKey::PopularTv => &mut self.home_popular_tv,
        }
    }

    fn load_more_home_shelf(&mut self, key: HomeShelfKey, cx: &mut Context<Self>) {
        let page = {
            let shelf = self.home_shelf_mut(key);
            if !shelf.can_load_more() || shelf.loading_more {
                return;
            }
            shelf.loading_more = true;
            shelf.load_more_error = None;
            shelf.page.saturating_add(1)
        };

        let generation = self.home_catalogue_generation;
        let metadata = self.metadata.clone();
        let artwork = self.artwork.clone();
        let cancellation = self.home_cancellation.child_token();
        let completion_cancellation = cancellation.clone();
        let task = self.tokio.spawn(async move {
            let response = match key {
                HomeShelfKey::Trending => metadata.trending(page, "en-US", &cancellation).await,
                HomeShelfKey::PopularMovies => {
                    metadata.popular_movies(page, "en-US", &cancellation).await
                }
                HomeShelfKey::PopularTv => metadata.popular_tv(page, "en-US", &cancellation).await,
            }
            .map_err(|error| metadata_error_message(&error))?;

            if response.page != page {
                return Err(format!(
                    "The catalogue returned page {} while page {} was requested. Retry this shelf.",
                    response.page, page
                ));
            }

            let total_pages = bounded_home_total_pages(page, response.total_pages);
            let fallback_type = match key {
                HomeShelfKey::Trending => None,
                HomeShelfKey::PopularMovies => Some(MediaType::Movie),
                HomeShelfKey::PopularTv => Some(MediaType::Tv),
            };
            let candidates = home_media_candidates(response.results, fallback_type);
            let mut cards = candidates
                .iter()
                .map(|candidate| HomeMediaCard {
                    id: candidate.id,
                    media_type: candidate.media_type,
                    title: candidate.title.clone(),
                    detail: candidate.detail.clone(),
                    poster: None,
                })
                .collect::<Vec<_>>();

            let mut jobs = JoinSet::new();
            for (index, candidate) in candidates.iter().enumerate() {
                let service = artwork.clone();
                let token = cancellation.child_token();
                let poster_path = candidate.poster_path.clone();
                jobs.spawn(async move {
                    let poster =
                        load_gpui_artwork(service, poster_path, ArtworkSize::W342, token).await;
                    (index, poster)
                });
            }

            while let Some(result) = jobs.join_next().await {
                if cancellation.is_cancelled() {
                    return Err("Cancelled".to_owned());
                }
                if let Ok((index, poster)) = result
                    && let Some(card) = cards.get_mut(index)
                {
                    card.poster = poster;
                }
            }

            if cancellation.is_cancelled() {
                return Err("Cancelled".to_owned());
            }

            Ok::<_, String>(HomeShelfPageData {
                page,
                total_pages,
                cards,
            })
        });

        cx.spawn(async move |this, cx| {
            let result = task.await.unwrap_or_else(|_| {
                Err("The next catalogue page stopped unexpectedly. Please retry.".to_owned())
            });
            let _ = this.update(cx, |this, cx| {
                if this.home_catalogue_generation != generation
                    || completion_cancellation.is_cancelled()
                {
                    return;
                }

                let shelf = this.home_shelf_mut(key);
                shelf.loading_more = false;
                match result {
                    Ok(data) => {
                        append_unique_home_cards(&mut shelf.cards, data.cards);
                        shelf.page = data.page;
                        shelf.total_pages = data.total_pages;
                        shelf.load_more_error = None;
                    }
                    Err(message) => {
                        shelf.load_more_error = Some(message);
                    }
                }
                cx.notify();
            });
        })
        .detach();

        cx.notify();
    }

    fn move_home_section(&mut self, key: &str, direction: isize, cx: &mut Context<Self>) {
        if self.home_settings.move_section(key, direction) {
            let has_hero_sources = self
                .home_settings
                .sections
                .iter()
                .any(|section| section.enabled && section.hero_source_enabled);
            self.persist_home_settings(cx);
            if has_hero_sources {
                self.refresh_home_catalogue(cx);
            }
        }
    }

    fn render_home_layout_settings(&self, cx: &mut Context<Self>) -> gpui::Div {
        let hero_enabled = self.home_settings.hero_enabled;
        let mut panel = div()
            .w_full()
            .flex()
            .flex_col()
            .gap_3()
            .rounded_lg()
            .border_1()
            .border_color(rgb(Theme::BORDER))
            .bg(rgb(Theme::SURFACE))
            .p_4()
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(Theme::TEXT_PRIMARY))
                    .child("HOME LAYOUT"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(Theme::TEXT_MUTED))
                    .child("Choose visible shelves, arrange their order, and select up to two sources for the featured carousel. Preferences are saved locally."),
            )
            .child(
                div()
                    .id("settings-home-hero-toggle")
                    .tab_index(42)
                    .cursor_pointer()
                    .rounded_md()
                    .bg(rgb(Theme::SURFACE_RAISED))
                    .px_3()
                    .py_3()
                    .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                    .aria_label(if hero_enabled { "Disable Home hero" } else { "Enable Home hero" })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_home_hero(cx);
                    }))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_PRIMARY))
                            .child(if hero_enabled { "Featured hero  ·  ON" } else { "Featured hero  ·  OFF" }),
                    ),
            );

        for (index, section) in self.home_settings.sections.iter().enumerate() {
            let key = section.key.clone();
            let title = section.display_title().to_owned();
            let enabled = section.enabled;
            let hero_source_enabled = section.hero_source_enabled;
            let base_tab = 50 + (index as isize * 4);

            let row = div()
                .w_full()
                .flex()
                .items_center()
                .gap_2()
                .rounded_md()
                .border_1()
                .border_color(rgb(Theme::BORDER))
                .bg(rgb(Theme::SURFACE_RAISED))
                .p_2()
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(Theme::TEXT_PRIMARY))
                                .child(title.clone()),
                        )
                        .child(div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child(
                            if enabled {
                                "Visible on Home"
                            } else {
                                "Hidden from Home"
                            },
                        )),
                )
                .child(
                    div()
                        .id(format!("settings-home-toggle-{}", key))
                        .tab_index(base_tab)
                        .cursor_pointer()
                        .rounded_md()
                        .bg(rgb(Theme::ACCENT_DARK))
                        .px_2()
                        .py_2()
                        .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                        .aria_label(if enabled {
                            format!("Hide {}", title)
                        } else {
                            format!("Show {}", title)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_home_section(&key, cx);
                        }))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(Theme::ACCENT))
                                .child(if enabled { "HIDE" } else { "SHOW" }),
                        ),
                );

            let row = if enabled {
                let hero_key = section.key.clone();
                row.child(
                    div()
                        .id(format!("settings-home-hero-source-{}", hero_key))
                        .tab_index(base_tab + 3)
                        .cursor_pointer()
                        .rounded_md()
                        .bg(rgb(if hero_source_enabled {
                            Theme::ACCENT_DARK
                        } else {
                            Theme::SURFACE
                        }))
                        .px_2()
                        .py_2()
                        .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                        .aria_label(if hero_source_enabled {
                            format!("Remove {} from featured hero sources", title)
                        } else {
                            format!("Use {} as a featured hero source", title)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_home_hero_source(&hero_key, cx);
                        }))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(if hero_source_enabled {
                                    Theme::ACCENT
                                } else {
                                    Theme::TEXT_SECONDARY
                                }))
                                .child(if hero_source_enabled {
                                    "HERO ON"
                                } else {
                                    "HERO OFF"
                                }),
                        ),
                )
            } else {
                row.child(
                    div().rounded_md().px_2().py_2().child(
                        div()
                            .text_xs()
                            .text_color(rgb(Theme::TEXT_MUTED))
                            .child("HERO OFF"),
                    ),
                )
            };

            let up_key = section.key.clone();
            let row = row.child(
                div()
                    .id(format!("settings-home-up-{}", up_key))
                    .tab_index(base_tab + 1)
                    .cursor_pointer()
                    .rounded_md()
                    .px_2()
                    .py_2()
                    .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                    .aria_label(format!("Move {} up", title))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.move_home_section(&up_key, -1, cx);
                    }))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_PRIMARY))
                            .child("↑"),
                    ),
            );

            let down_key = section.key.clone();
            let row = row.child(
                div()
                    .id(format!("settings-home-down-{}", down_key))
                    .tab_index(base_tab + 2)
                    .cursor_pointer()
                    .rounded_md()
                    .px_2()
                    .py_2()
                    .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                    .aria_label(format!("Move {} down", title))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.move_home_section(&down_key, 1, cx);
                    }))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_PRIMARY))
                            .child("↓"),
                    ),
            );

            panel = panel.child(row);
        }

        panel
    }
    fn load_home_catalogue(&mut self, cx: &mut Context<Self>) {
        let all_shelves_recovered = [
            &self.home_trending,
            &self.home_popular_movies,
            &self.home_popular_tv,
        ]
        .iter()
        .all(|shelf| shelf.error.is_none());
        if self.home_catalogue_status == HomeCatalogueStatus::Loading
            || (self.home_catalogue_status == HomeCatalogueStatus::Loaded && all_shelves_recovered)
        {
            return;
        }

        self.home_catalogue_generation = self.home_catalogue_generation.wrapping_add(1);
        let generation = self.home_catalogue_generation;
        self.home_catalogue_status = HomeCatalogueStatus::Loading;
        for shelf in [
            &mut self.home_trending,
            &mut self.home_popular_movies,
            &mut self.home_popular_tv,
        ] {
            shelf.status = HomeCatalogueStatus::Loading;
            shelf.error = None;
            shelf.loading_more = false;
            shelf.load_more_error = None;
        }

        let metadata = self.metadata.clone();
        let artwork = self.artwork.clone();
        let hero_sources = selected_home_hero_sources(&self.home_settings);
        let cancellation = self.home_cancellation.child_token();
        let completion_cancellation = cancellation.clone();
        let task = self.tokio.spawn(async move {
            let (trending_response, movie_response, tv_response) = tokio::join!(
                metadata.trending(1, "en-US", &cancellation),
                metadata.popular_movies(1, "en-US", &cancellation),
                metadata.popular_tv(1, "en-US", &cancellation),
            );

            let mut trending = match trending_response {
                Ok(response) => {
                    let page = response.page.clamp(1, MAX_HOME_CATALOGUE_PAGES);
                    home_shelf_load_data(
                        home_media_candidates(response.results, None),
                        None,
                        page,
                        response.total_pages,
                    )
                }
                Err(error) => {
                    home_shelf_load_data(Vec::new(), Some(metadata_error_message(&error)), 0, None)
                }
            };
            let mut popular_movies = match movie_response {
                Ok(response) => {
                    let page = response.page.clamp(1, MAX_HOME_CATALOGUE_PAGES);
                    home_shelf_load_data(
                        home_media_candidates(response.results, Some(MediaType::Movie)),
                        None,
                        page,
                        response.total_pages,
                    )
                }
                Err(error) => {
                    home_shelf_load_data(Vec::new(), Some(metadata_error_message(&error)), 0, None)
                }
            };
            let mut popular_tv = match tv_response {
                Ok(response) => {
                    let page = response.page.clamp(1, MAX_HOME_CATALOGUE_PAGES);
                    home_shelf_load_data(
                        home_media_candidates(response.results, Some(MediaType::Tv)),
                        None,
                        page,
                        response.total_pages,
                    )
                }
                Err(error) => {
                    home_shelf_load_data(Vec::new(), Some(metadata_error_message(&error)), 0, None)
                }
            };

            let mut featured_candidates = Vec::with_capacity(MAX_HOME_FEATURED_ITEMS);
            for key in hero_sources {
                let candidates = match key {
                    HomeShelfKey::Trending => &trending.candidates,
                    HomeShelfKey::PopularMovies => &popular_movies.candidates,
                    HomeShelfKey::PopularTv => &popular_tv.candidates,
                };
                for (index, candidate) in candidates.iter().enumerate() {
                    if candidate.backdrop_path.is_none() {
                        continue;
                    }
                    featured_candidates.push((key, index, candidate.clone()));
                    if featured_candidates.len() >= MAX_HOME_FEATURED_ITEMS {
                        break;
                    }
                }
                if featured_candidates.len() >= MAX_HOME_FEATURED_ITEMS {
                    break;
                }
            }
            let featured_indices = featured_candidates
                .iter()
                .map(|(key, index, _)| (*key, *index))
                .collect::<Vec<_>>();

            let mut jobs = JoinSet::new();
            for (key, candidates) in [
                (HomeShelfKey::Trending, &trending.candidates),
                (HomeShelfKey::PopularMovies, &popular_movies.candidates),
                (HomeShelfKey::PopularTv, &popular_tv.candidates),
            ] {
                for (index, candidate) in candidates.iter().enumerate() {
                    let service = artwork.clone();
                    let task_token = cancellation.child_token();
                    let poster_path = candidate.poster_path.clone();
                    let backdrop_path = if featured_indices.contains(&(key, index)) {
                        candidate.backdrop_path.clone()
                    } else {
                        None
                    };
                    jobs.spawn(async move {
                        let poster = load_gpui_artwork(
                            service.clone(),
                            poster_path,
                            ArtworkSize::W342,
                            task_token.child_token(),
                        )
                        .await;
                        let backdrop = match backdrop_path {
                            Some(path) => {
                                load_gpui_artwork(
                                    service,
                                    path,
                                    ArtworkSize::W1280,
                                    task_token.child_token(),
                                )
                                .await
                            }
                            None => None,
                        };
                        (key, index, poster, backdrop)
                    });
                }
            }

            let mut featured_backdrops = HashMap::new();
            while let Some(result) = jobs.join_next().await {
                if cancellation.is_cancelled() {
                    return Err("Cancelled".to_owned());
                }
                if let Ok((key, index, poster, backdrop)) = result {
                    let shelf = match key {
                        HomeShelfKey::Trending => &mut trending,
                        HomeShelfKey::PopularMovies => &mut popular_movies,
                        HomeShelfKey::PopularTv => &mut popular_tv,
                    };
                    if let Some(card) = shelf.cards.get_mut(index) {
                        card.poster = poster;
                    }
                    if let Some(backdrop) = backdrop {
                        featured_backdrops.insert((key, index), backdrop);
                    }
                }
            }

            if cancellation.is_cancelled() {
                return Err("Cancelled".to_owned());
            }

            let mut featured = Vec::with_capacity(featured_candidates.len());
            for (key, index, candidate) in featured_candidates {
                let Some(backdrop) = featured_backdrops.remove(&(key, index)) else {
                    continue;
                };
                let shelf = match key {
                    HomeShelfKey::Trending => &trending,
                    HomeShelfKey::PopularMovies => &popular_movies,
                    HomeShelfKey::PopularTv => &popular_tv,
                };
                let poster = shelf.cards.get(index).and_then(|card| card.poster.clone());
                featured.push(HomeFeaturedCard {
                    id: candidate.id,
                    media_type: candidate.media_type,
                    title: candidate.title,
                    detail: candidate.detail,
                    overview: candidate.overview,
                    poster,
                    backdrop,
                });
            }

            Ok::<_, String>(HomeCatalogueData {
                trending,
                popular_movies,
                popular_tv,
                featured,
            })
        });

        cx.spawn(async move |this, cx| {
            let result = task.await.unwrap_or_else(|_| {
                Err("The Home catalogue request stopped unexpectedly. Check your connection and retry."
                    .to_owned())
            });
            let _ = this.update(cx, |this, cx| {
                if this.home_catalogue_generation != generation
                    || completion_cancellation.is_cancelled()
                {
                    return;
                }
                match result {
                    Ok(data) => {
                        this.home_trending = HomeShelfState {
                            status: HomeCatalogueStatus::Loaded,
                            error: data.trending.error,
                            cards: data.trending.cards,
                            page: data.trending.page,
                            total_pages: data.trending.total_pages,
                            loading_more: false,
                            load_more_error: None,
                        };
                        this.home_popular_movies = HomeShelfState {
                            status: HomeCatalogueStatus::Loaded,
                            error: data.popular_movies.error,
                            cards: data.popular_movies.cards,
                            page: data.popular_movies.page,
                            total_pages: data.popular_movies.total_pages,
                            loading_more: false,
                            load_more_error: None,
                        };
                        this.home_popular_tv = HomeShelfState {
                            status: HomeCatalogueStatus::Loaded,
                            error: data.popular_tv.error,
                            cards: data.popular_tv.cards,
                            page: data.popular_tv.page,
                            total_pages: data.popular_tv.total_pages,
                            loading_more: false,
                            load_more_error: None,
                        };
                        this.home_featured_items = data.featured;
                        this.home_featured_index = 0;
                        this.home_catalogue_status = HomeCatalogueStatus::Loaded;
                    }
                    Err(message) => {
                        for shelf in [
                            &mut this.home_trending,
                            &mut this.home_popular_movies,
                            &mut this.home_popular_tv,
                        ] {
                            shelf.status = HomeCatalogueStatus::Loaded;
                            shelf.error = Some(message.clone());
                            shelf.cards.clear();
                            shelf.page = 0;
                            shelf.total_pages = 1;
                            shelf.loading_more = false;
                            shelf.load_more_error = None;
                        }
                        this.home_featured_items.clear();
                        this.home_featured_index = 0;
                        this.home_catalogue_status = HomeCatalogueStatus::Loaded;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn step_home_featured(&mut self, direction: isize, cx: &mut Context<Self>) {
        self.home_featured_index = next_home_featured_index(
            self.home_featured_index,
            self.home_featured_items.len(),
            direction,
        );
        cx.notify();
    }

    fn open_home_card(
        &mut self,
        id: u64,
        media_type: MediaType,
        title: String,
        poster: Option<Arc<RenderImage>>,
        cx: &mut Context<Self>,
    ) {
        match media_type {
            MediaType::Person => self.load_person_details(id, title, poster, cx),
            MediaType::Movie | MediaType::Tv => self.load_details(id, media_type, title, cx),
            MediaType::Other => {}
        }
    }

    fn open_details(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(card) = self.search_results.get(index) else {
            return;
        };
        if !matches!(card.media_type, MediaType::Movie | MediaType::Tv) {
            return;
        }
        self.load_details(card.id, card.media_type, card.title.clone(), cx);
    }

    fn load_details(
        &mut self,
        id: u64,
        media_type: MediaType,
        initial_title: String,
        cx: &mut Context<Self>,
    ) {
        self.person_details_cancellation.cancel();
        self.person_details = None;
        self.details_cancellation.cancel();
        let cancellation = CancellationToken::new();
        self.details_cancellation = cancellation.clone();
        self.details = Some(DetailsPage {
            id,
            media_type,
            title: initial_title,
            overview: None,
            tagline: None,
            release_date: None,
            runtime: None,
            status: None,
            rating: None,
            genres: Vec::new(),
            cast: Vec::new(),
            seasons: Vec::new(),
            selected_season: None,
            selected_season_number: None,
            season_loading: false,
            season_error: None,
            season_cancellation: CancellationToken::new(),
            poster: None,
            backdrop: None,
            loading: true,
            error: None,
        });
        let metadata = self.metadata.clone();
        let artwork = self.artwork.clone();
        let task_cancellation = cancellation.clone();
        let task = self.tokio.spawn(async move {
            let cancellation = task_cancellation;
            let data = match media_type {
                MediaType::Movie => {
                    let m = metadata
                        .movie(id, &cancellation)
                        .await
                        .map_err(|error| metadata_error_message(&error))?;
                    let cast = m
                        .credits
                        .as_ref()
                        .map(|c| {
                            c.cast
                                .iter()
                                .take(12)
                                .map(|p| {
                                    (
                                        p.name.clone(),
                                        p.character.clone().unwrap_or_else(|| "Cast".into()),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    (
                        m.title,
                        m.overview,
                        m.tagline,
                        m.release_date,
                        m.runtime,
                        m.status,
                        m.vote_average,
                        m.genres.into_iter().map(|g| g.name).collect(),
                        cast,
                        Vec::new(),
                        m.poster_path,
                        m.backdrop_path,
                    )
                }
                MediaType::Tv => {
                    let t = metadata
                        .tv(id, &cancellation)
                        .await
                        .map_err(|error| metadata_error_message(&error))?;
                    let cast = t
                        .credits
                        .as_ref()
                        .or(t.aggregate_credits.as_ref())
                        .map(|c| {
                            c.cast
                                .iter()
                                .take(12)
                                .map(|p| {
                                    (
                                        p.name.clone(),
                                        p.character.clone().unwrap_or_else(|| "Cast".into()),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let runtime = t.episode_run_time.as_ref().and_then(|v| v.first()).copied();
                    (
                        t.name,
                        t.overview,
                        t.tagline,
                        t.first_air_date,
                        runtime,
                        t.status,
                        t.vote_average,
                        t.genres.into_iter().map(|g| g.name).collect(),
                        cast,
                        t.seasons,
                        t.poster_path,
                        t.backdrop_path,
                    )
                }
                _ => return Err("Unsupported media type".to_owned()),
            };
            let (
                title,
                overview,
                tagline,
                date,
                runtime,
                status,
                rating,
                genres,
                cast,
                seasons,
                poster_path,
                backdrop_path,
            ) = data;
            let poster = match poster_path {
                Some(path) => {
                    load_gpui_artwork(
                        artwork.clone(),
                        path,
                        ArtworkSize::W500,
                        cancellation.child_token(),
                    )
                    .await
                }
                None => None,
            };
            let backdrop = match backdrop_path {
                Some(path) => {
                    load_gpui_artwork(
                        artwork,
                        path,
                        ArtworkSize::W1280,
                        cancellation.child_token(),
                    )
                    .await
                }
                None => None,
            };
            if cancellation.is_cancelled() {
                return Err("Cancelled".to_owned());
            }
            Ok::<_, String>((
                title, overview, tagline, date, runtime, status, rating, genres, cast, seasons,
                poster, backdrop,
            ))
        });
        cx.spawn(async move |this, cx| {
            let result =
                task.await.unwrap_or_else(|_| {
                    Err("The details request stopped unexpectedly. Check your connection and retry."
                    .to_owned())
                });
            let _ = this.update(cx, |this, cx| {
                if cancellation.is_cancelled() {
                    return;
                }
                let Some(d) = this
                    .details
                    .as_mut()
                    .filter(|d| d.id == id && d.media_type == media_type)
                else {
                    return;
                };
                match result {
                    Ok((
                        title,
                        overview,
                        tagline,
                        date,
                        runtime,
                        status,
                        rating,
                        genres,
                        cast,
                        seasons,
                        poster,
                        backdrop,
                    )) => {
                        d.title = title;
                        d.overview = overview;
                        d.tagline = tagline;
                        d.release_date = date;
                        d.runtime = runtime;
                        d.status = status;
                        d.rating = rating;
                        d.genres = genres;
                        d.cast = cast;
                        d.seasons = seasons;
                        d.poster = poster;
                        d.backdrop = backdrop;
                        d.loading = false;
                        d.error = None;
                    }
                    Err(error) => {
                        d.loading = false;
                        d.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn retry_details(&mut self, cx: &mut Context<Self>) {
        let Some(details) = &self.details else {
            return;
        };
        let id = details.id;
        let media_type = details.media_type;
        let title = details.title.clone();
        self.load_details(id, media_type, title, cx);
    }

    fn close_details(&mut self, cx: &mut Context<Self>) {
        self.details_cancellation.cancel();
        self.details = None;
        cx.notify();
    }

    fn load_detail_season(&mut self, number: u32, cx: &mut Context<Self>) {
        let Some(current) = self.details.as_ref() else {
            return;
        };
        if current.media_type != MediaType::Tv
            || current.loading
            || current.selected_season_number == Some(number) && current.selected_season.is_some()
        {
            return;
        }

        let id = current.id;
        let cancellation = self.details_cancellation.child_token();
        if let Some(details) = self.details.as_mut() {
            details.season_cancellation.cancel();
            details.season_cancellation = cancellation.clone();
            details.selected_season_number = Some(number);
            details.selected_season = None;
            details.season_loading = true;
            details.season_error = None;
        }

        let task_token = cancellation.clone();
        let metadata = self.metadata.clone();
        let task = self.tokio.spawn(async move {
            metadata
                .season(id, number, &task_token)
                .await
                .map_err(|error| metadata_error_message(&error))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await.unwrap_or_else(|_| {
                Err(
                    "The season request stopped unexpectedly. Check your connection and retry."
                        .to_owned(),
                )
            });
            let _ = this.update(cx, |this, cx| {
                if cancellation.is_cancelled() {
                    return;
                }
                if let Some(details) = this.details.as_mut().filter(|details| {
                    details.id == id && details.selected_season_number == Some(number)
                }) {
                    details.season_loading = false;
                    match result {
                        Ok(season) => {
                            details.selected_season = Some(season);
                            details.season_error = None;
                        }
                        Err(error) => {
                            details.season_error = Some(error);
                        }
                    }
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn render_details(&self, cx: &mut Context<Self>) -> gpui::Div {
        let Some(d) = &self.details else {
            return div();
        };
        let mut page = div().w_full().flex().flex_col().gap_5().child(
            div()
                .id("details-back")
                .tab_index(20)
                .cursor_pointer()
                .rounded_md()
                .bg(rgb(Theme::SURFACE))
                .px_3()
                .py_2()
                .on_click(cx.listener(|this, _, _, cx| this.close_details(cx)))
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(Theme::ACCENT))
                        .child("← Back to results"),
                ),
        );
        if d.loading {
            return page
                .child(backdrop_surface(None, 260.0, true))
                .child(status_panel(
                    StatusTone::Loading,
                    "LOADING DETAILS",
                    "Fetching title information",
                    "Retrieving full metadata and artwork.",
                ));
        }
        if let Some(error) = &d.error {
            page = page.child(status_panel(
                StatusTone::Error,
                "DETAILS UNAVAILABLE",
                "Could not load this title",
                error,
            ));
            return page.child(
                div()
                    .id("retry-details")
                    .tab_index(21)
                    .cursor_pointer()
                    .rounded_md()
                    .bg(rgb(Theme::ACCENT_DARK))
                    .px_3()
                    .py_2()
                    .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                    .aria_label("Retry loading title details")
                    .on_click(cx.listener(|this, _, _, cx| this.retry_details(cx)))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::ACCENT))
                            .child("Retry"),
                    ),
            );
        }
        page = page.child(backdrop_surface(d.backdrop.clone(), 260.0, false));
        let mut hero = div().w_full().flex().items_start().gap_5();
        if let Some(poster) = d.poster.clone() {
            hero = hero.child(
                div()
                    .w(px(164.0))
                    .h(px(246.0))
                    .rounded_lg()
                    .overflow_hidden()
                    .child(img(poster).w_full().h_full()),
            );
        }
        let label = if d.media_type == MediaType::Movie {
            "MOVIE"
        } else {
            "TV SERIES"
        };
        let mut info = div()
            .flex_1()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_xs().text_color(rgb(Theme::ACCENT)).child(label))
            .child(
                div()
                    .text_2xl()
                    .text_color(rgb(Theme::TEXT_PRIMARY))
                    .child(d.title.clone()),
            );
        if let Some(tagline) = &d.tagline {
            info = info.child(
                div()
                    .text_sm()
                    .text_color(rgb(Theme::TEXT_SECONDARY))
                    .child(tagline.clone()),
            );
        }
        let mut facts = Vec::new();
        if let Some(date) = &d.release_date {
            facts.push(date.clone());
        }
        if let Some(runtime) = d.runtime {
            facts.push(format!("{runtime} min"));
        }
        if let Some(status) = &d.status {
            facts.push(status.clone());
        }
        if let Some(rating) = d.rating {
            facts.push(format!("★ {rating:.1}/10"));
        }
        if !facts.is_empty() {
            info = info.child(
                div()
                    .text_sm()
                    .text_color(rgb(Theme::TEXT_MUTED))
                    .child(facts.join(" · ")),
            );
        }
        if !d.genres.is_empty() {
            info = info.child(
                div()
                    .text_sm()
                    .text_color(rgb(Theme::ACCENT))
                    .child(d.genres.join(" · ")),
            );
        }
        page = page.child(hero.child(info));
        page = page.child(
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_lg()
                        .text_color(rgb(Theme::TEXT_PRIMARY))
                        .child("Overview"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(Theme::TEXT_SECONDARY))
                        .child(
                            d.overview
                                .as_deref()
                                .filter(|v| !v.trim().is_empty())
                                .unwrap_or("No synopsis is available.")
                                .to_owned(),
                        ),
                ),
        );
        if !d.cast.is_empty() {
            let mut row = div().w_full().flex().flex_wrap().gap_3();
            for (name, role) in d.cast.iter().take(12) {
                row = row.child(
                    div()
                        .rounded_md()
                        .bg(rgb(Theme::SURFACE))
                        .p_3()
                        .min_w(px(140.0))
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(Theme::TEXT_PRIMARY))
                                .child(name.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(Theme::TEXT_MUTED))
                                .child(role.clone()),
                        ),
                );
            }
            page = page.child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_lg()
                            .text_color(rgb(Theme::TEXT_PRIMARY))
                            .child("Cast"),
                    )
                    .child(row),
            );
        }
        if d.media_type == MediaType::Tv && !d.seasons.is_empty() {
            let mut selector = div().flex().flex_wrap().gap_2();
            for season in &d.seasons {
                let number = season.season_number;
                let selected = d.selected_season_number == Some(number);
                selector = selector.child(
                    div()
                        .id(format!("season-{number}"))
                        .tab_index(30 + number as isize)
                        .cursor_pointer()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(if selected {
                            Theme::ACCENT
                        } else {
                            Theme::BORDER
                        }))
                        .bg(rgb(Theme::SURFACE))
                        .px_3()
                        .py_2()
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.load_detail_season(number, cx)),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(if selected {
                                    Theme::ACCENT
                                } else {
                                    Theme::TEXT_PRIMARY
                                }))
                                .child(season.name.clone()),
                        ),
                );
            }
            let mut section = div()
                .w_full()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .text_lg()
                        .text_color(rgb(Theme::TEXT_PRIMARY))
                        .child("Seasons"),
                )
                .child(selector);
            if d.season_loading {
                section = section.child(status_panel(
                    StatusTone::Loading,
                    "SEASON",
                    "Loading episodes",
                    "Fetching the selected season.",
                ));
            } else if let (Some(error), Some(number)) = (&d.season_error, d.selected_season_number)
            {
                section = section.child(status_panel(
                    StatusTone::Error,
                    "SEASON UNAVAILABLE",
                    "Could not load these episodes",
                    error,
                ));
                section = section.child(
                    div()
                        .id("retry-detail-season")
                        .tab_index(50)
                        .rounded_md()
                        .bg(rgb(Theme::ACCENT_DARK))
                        .px_3()
                        .py_2()
                        .cursor_pointer()
                        .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                        .aria_label("Retry loading this season")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.load_detail_season(number, cx);
                        }))
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(Theme::ACCENT))
                                .child("Retry season"),
                        ),
                );
            } else if let Some(season) = &d.selected_season {
                let mut episodes = div().w_full().flex().flex_col().gap_2();
                for episode in &season.episodes {
                    let mut heading = format!(
                        "S{:02} E{:02} · {}",
                        episode.season_number, episode.episode_number, episode.name
                    );
                    if let Some(minutes) = episode.runtime {
                        heading.push_str(&format!(" · {minutes} min"));
                    }
                    episodes = episodes.child(
                        div()
                            .rounded_md()
                            .bg(rgb(Theme::SURFACE))
                            .p_3()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(Theme::TEXT_PRIMARY))
                                    .child(heading),
                            )
                            .child(
                                div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child(
                                    episode
                                        .overview
                                        .as_deref()
                                        .unwrap_or("No episode synopsis available.")
                                        .to_owned(),
                                ),
                            ),
                    );
                }
                section = section.child(episodes);
            }
            page = page.child(section);
        }
        page
    }

    fn open_person_details(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(card) = self.search_results.get(index) else {
            return;
        };
        if card.media_type != MediaType::Person {
            return;
        }
        self.load_person_details(card.id, card.title.clone(), card.poster.clone(), cx);
    }

    fn load_person_details(
        &mut self,
        id: u64,
        initial_name: String,
        initial_profile: Option<Arc<RenderImage>>,
        cx: &mut Context<Self>,
    ) {
        self.details_cancellation.cancel();
        self.details = None;
        self.person_details_cancellation.cancel();
        let cancellation = CancellationToken::new();
        self.person_details_cancellation = cancellation.clone();
        self.person_details = Some(PersonDetailsPage {
            id,
            name: initial_name,
            biography: None,
            birthday: None,
            deathday: None,
            place_of_birth: None,
            known_for_department: None,
            profile: initial_profile,
            movie_credits: Vec::new(),
            tv_credits: Vec::new(),
            loading: true,
            error: None,
        });

        let metadata = self.metadata.clone();
        let artwork = self.artwork.clone();
        let task_token = cancellation.clone();
        let task = self.tokio.spawn(async move {
            let person = metadata
                .person(id, "en-US", &task_token)
                .await
                .map_err(|error| metadata_error_message(&error))?;
            if person.id != id {
                return Err(
                    "The metadata service returned a different person than requested.".to_owned(),
                );
            }

            let profile = match person.profile_path.as_deref() {
                Some(path) => {
                    load_gpui_artwork(
                        artwork.clone(),
                        path.to_owned(),
                        ArtworkSize::W500,
                        task_token.child_token(),
                    )
                    .await
                }
                None => None,
            };

            let movie_candidates = person_credit_candidates(&person, MediaType::Movie);
            let tv_candidates = person_credit_candidates(&person, MediaType::Tv);
            let mut movie_credits: Vec<PersonCreditCard> = movie_candidates
                .iter()
                .map(|candidate| PersonCreditCard {
                    id: candidate.id,
                    media_type: candidate.media_type,
                    title: candidate.title.clone(),
                    detail: candidate.detail.clone(),
                    poster: None,
                })
                .collect();
            let mut tv_credits: Vec<PersonCreditCard> = tv_candidates
                .iter()
                .map(|candidate| PersonCreditCard {
                    id: candidate.id,
                    media_type: candidate.media_type,
                    title: candidate.title.clone(),
                    detail: candidate.detail.clone(),
                    poster: None,
                })
                .collect();

            let mut jobs = JoinSet::new();
            for (index, candidate) in movie_candidates.into_iter().enumerate() {
                let service = artwork.clone();
                let token = task_token.child_token();
                jobs.spawn(async move {
                    let image = load_gpui_artwork(
                        service,
                        candidate.artwork_path,
                        candidate.artwork_size,
                        token,
                    )
                    .await;
                    (MediaType::Movie, index, image)
                });
            }
            for (index, candidate) in tv_candidates.into_iter().enumerate() {
                let service = artwork.clone();
                let token = task_token.child_token();
                jobs.spawn(async move {
                    let image = load_gpui_artwork(
                        service,
                        candidate.artwork_path,
                        candidate.artwork_size,
                        token,
                    )
                    .await;
                    (MediaType::Tv, index, image)
                });
            }

            while let Some(result) = jobs.join_next().await {
                if task_token.is_cancelled() {
                    return Err("Cancelled".to_owned());
                }
                if let Ok((media_type, index, image)) = result {
                    let target = match media_type {
                        MediaType::Movie => &mut movie_credits,
                        MediaType::Tv => &mut tv_credits,
                        MediaType::Person | MediaType::Other => continue,
                    };
                    if let Some(card) = target.get_mut(index) {
                        card.poster = image;
                    }
                }
            }

            if task_token.is_cancelled() {
                return Err("Cancelled".to_owned());
            }

            Ok::<_, String>((
                person.name,
                person.biography,
                person.birthday,
                person.deathday,
                person.place_of_birth,
                person.known_for_department,
                profile,
                movie_credits,
                tv_credits,
            ))
        });

        cx.spawn(async move |this, cx| {
            let result = task.await.unwrap_or_else(|_| {
                Err("The person details request stopped unexpectedly. Check your connection and retry."
                    .to_owned())
            });
            let _ = this.update(cx, |this, cx| {
                if cancellation.is_cancelled() {
                    return;
                }
                let Some(page) = this
                    .person_details
                    .as_mut()
                    .filter(|page| page.id == id)
                else {
                    return;
                };
                match result {
                    Ok((
                        name,
                        biography,
                        birthday,
                        deathday,
                        place_of_birth,
                        known_for_department,
                        profile,
                        movie_credits,
                        tv_credits,
                    )) => {
                        page.name = name;
                        page.biography = biography;
                        page.birthday = birthday;
                        page.deathday = deathday;
                        page.place_of_birth = place_of_birth;
                        page.known_for_department = known_for_department;
                        if profile.is_some() {
                            page.profile = profile;
                        }
                        page.movie_credits = movie_credits;
                        page.tv_credits = tv_credits;
                        page.loading = false;
                        page.error = None;
                    }
                    Err(error) => {
                        page.loading = false;
                        page.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn retry_person_details(&mut self, cx: &mut Context<Self>) {
        let Some(page) = &self.person_details else {
            return;
        };
        self.load_person_details(page.id, page.name.clone(), page.profile.clone(), cx);
    }

    fn close_person_details(&mut self, cx: &mut Context<Self>) {
        self.person_details_cancellation.cancel();
        self.person_details = None;
        cx.notify();
    }

    fn render_person_credit_shelf(
        &self,
        key: &str,
        title: &str,
        credits: &[PersonCreditCard],
        tab_start: isize,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let mut row = div().flex().gap_4();
        for (index, credit) in credits.iter().enumerate() {
            let id = credit.id;
            let media_type = credit.media_type;
            let media_title = credit.title.clone();
            row = row.child(
                div()
                    .id(format!("person-{key}-{id}-{index}"))
                    .tab_index(tab_start + index as isize)
                    .cursor_pointer()
                    .rounded_lg()
                    .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                    .aria_label(format!("Open {}", credit.title))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.load_details(id, media_type, media_title.clone(), cx);
                    }))
                    .child(poster_card(
                        &credit.title,
                        &credit.detail,
                        credit.poster.clone(),
                        false,
                    )),
            );
        }

        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_lg()
                    .text_color(rgb(Theme::TEXT_PRIMARY))
                    .child(title.to_owned()),
            )
            .child(
                div()
                    .id(format!("person-{key}-shelf-scroll"))
                    .w_full()
                    .overflow_x_scroll()
                    .child(row),
            )
    }

    fn render_person_details(&self, cx: &mut Context<Self>) -> gpui::Div {
        let Some(person) = &self.person_details else {
            return div();
        };
        let mut page = div().w_full().flex().flex_col().gap_5().child(
            div()
                .id("person-details-back")
                .tab_index(20)
                .cursor_pointer()
                .rounded_md()
                .bg(rgb(Theme::SURFACE))
                .px_3()
                .py_2()
                .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                .aria_label("Back to search results")
                .on_click(cx.listener(|this, _, _, cx| this.close_person_details(cx)))
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(Theme::ACCENT))
                        .child("← Back to results"),
                ),
        );

        if person.loading {
            return page
                .child(status_panel(
                    StatusTone::Loading,
                    "PERSON",
                    "Loading person details",
                    "Fetching profile information, filmography and validated artwork.",
                ))
                .child(backdrop_surface(None, 180.0, true));
        }
        if let Some(error) = &person.error {
            page = page.child(status_panel(
                StatusTone::Error,
                "PERSON UNAVAILABLE",
                "Could not load this person's details",
                error,
            ));
            return page.child(
                div()
                    .id("retry-person-details")
                    .tab_index(21)
                    .rounded_md()
                    .bg(rgb(Theme::ACCENT_DARK))
                    .px_3()
                    .py_2()
                    .cursor_pointer()
                    .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                    .aria_label("Retry loading person details")
                    .on_click(cx.listener(|this, _, _, cx| this.retry_person_details(cx)))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::ACCENT))
                            .child("Retry"),
                    ),
            );
        }

        let mut identity = div().w_full().flex().items_start().gap_5();
        if let Some(profile) = person.profile.clone() {
            identity = identity.child(
                div()
                    .w(px(180.0))
                    .h(px(270.0))
                    .rounded_lg()
                    .overflow_hidden()
                    .bg(rgb(Theme::SURFACE_RAISED))
                    .child(img(profile).w_full().h_full()),
            );
        } else {
            identity = identity.child(poster_card(&person.name, "PERSON", None, false));
        }

        let mut facts = Vec::new();
        if let Some(birthday) = &person.birthday {
            facts.push(format!("Born {birthday}"));
        }
        if let Some(deathday) = &person.deathday {
            facts.push(format!("Died {deathday}"));
        }
        if let Some(place) = &person.place_of_birth
            && !place.trim().is_empty()
        {
            facts.push(place.clone());
        }
        let mut info = div()
            .flex_1()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div().text_xs().text_color(rgb(Theme::ACCENT)).child(
                    person
                        .known_for_department
                        .as_deref()
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or("PERSON")
                        .to_uppercase(),
                ),
            )
            .child(
                div()
                    .text_2xl()
                    .text_color(rgb(Theme::TEXT_PRIMARY))
                    .child(person.name.clone()),
            );
        if !facts.is_empty() {
            info = info.child(
                div()
                    .text_sm()
                    .text_color(rgb(Theme::TEXT_MUTED))
                    .child(facts.join(" · ")),
            );
        }
        page = page.child(identity.child(info));

        page = page.child(
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_lg()
                        .text_color(rgb(Theme::TEXT_PRIMARY))
                        .child("Biography"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(Theme::TEXT_SECONDARY))
                        .child(
                            person
                                .biography
                                .as_deref()
                                .filter(|value| !value.trim().is_empty())
                                .unwrap_or("No biography is available for this person.")
                                .to_owned(),
                        ),
                ),
        );

        if person.movie_credits.is_empty() && person.tv_credits.is_empty() {
            page = page.child(status_panel(
                StatusTone::Empty,
                "FILMOGRAPHY",
                "No credited titles are available",
                "The person profile loaded, but the metadata provider returned no movie or TV credits with artwork.",
            ));
        } else {
            if !person.movie_credits.is_empty() {
                page = page.child(self.render_person_credit_shelf(
                    "movies",
                    "Movie credits",
                    &person.movie_credits,
                    80,
                    cx,
                ));
            }
            if !person.tv_credits.is_empty() {
                page = page.child(self.render_person_credit_shelf(
                    "tv",
                    "TV credits",
                    &person.tv_credits,
                    110,
                    cx,
                ));
            }
        }
        page
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
                    let message = metadata_error_message(&error);
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
                Err(_) => {
                    let message = concat!(
                        "The search request stopped before a response was received. ",
                        "Check your connection and retry.",
                    )
                    .to_owned();
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

                if let Some(index) = self.selected_search_result
                    && let Some(card) = self.search_results.get(index)
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
                    if card.media_type == MediaType::Person {
                        preview = preview.child(
                            div()
                                .id("open-person-details")
                                .tab_index(61)
                                .cursor_pointer()
                                .rounded_md()
                                .bg(rgb(Theme::ACCENT_DARK))
                                .px_3()
                                .py_2()
                                .focus_visible(|style| {
                                    style.border_1().border_color(rgb(Theme::ACCENT))
                                })
                                .aria_label(format!("View details for {}", card.title))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_person_details(index, cx);
                                }))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(Theme::ACCENT))
                                        .child("View person details"),
                                ),
                        );
                    } else if matches!(card.media_type, MediaType::Movie | MediaType::Tv) {
                        preview = preview.child(
                            div()
                                .id("open-title-details")
                                .tab_index(60)
                                .cursor_pointer()
                                .rounded_md()
                                .bg(rgb(Theme::ACCENT_DARK))
                                .px_3()
                                .py_2()
                                .focus_visible(|style| {
                                    style.border_1().border_color(rgb(Theme::ACCENT))
                                })
                                .aria_label(format!("View details for {}", card.title))
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.open_details(index, cx)),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(Theme::ACCENT))
                                        .child("View full details"),
                                ),
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

    fn render_home_shelf(
        &self,
        key: HomeShelfKey,
        title: &str,
        shelf: &HomeShelfState,
        tab_start: isize,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let content = if shelf.cards.is_empty()
            && matches!(
                shelf.status,
                HomeCatalogueStatus::NotRequested | HomeCatalogueStatus::Loading
            ) {
            let mut row = div().flex().gap_4();
            for _ in 0..7 {
                row = row.child(poster_card(" ", " ", None, true));
            }
            row
        } else if shelf.cards.is_empty() {
            div().w_full().child(status_panel(
                StatusTone::Empty,
                "NO TITLES AVAILABLE",
                "Nothing to show yet",
                "This catalogue did not return titles with usable poster artwork.",
            ))
        } else {
            let mut row = div().flex().gap_4();
            for (index, card) in shelf.cards.iter().enumerate() {
                let id = card.id;
                let media_type = card.media_type;
                let media_title = card.title.clone();
                let poster = card.poster.clone();
                row = row.child(
                    div()
                        .id(format!(
                            "home-{}-{}-{}",
                            key.as_str(),
                            home_media_type_key(card.media_type),
                            card.id
                        ))
                        .tab_index(tab_start + index as isize)
                        .cursor_pointer()
                        .rounded_lg()
                        .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                        .aria_label(format!("Open {}", card.title))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_home_card(
                                id,
                                media_type,
                                media_title.clone(),
                                poster.clone(),
                                cx,
                            );
                        }))
                        .child(poster_card(
                            &card.title,
                            &card.detail,
                            card.poster.clone(),
                            false,
                        )),
                );
            }
            row
        };

        let mut section = div()
            .w_full()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_lg()
                    .text_color(rgb(Theme::TEXT_PRIMARY))
                    .child(title.to_owned()),
            )
            .child(
                div()
                    .id(format!("home-{}-shelf-scroll", key.as_str()))
                    .w_full()
                    .overflow_x_scroll()
                    .child(content),
            );

        if let Some(error) = &shelf.error {
            section = section.child(status_panel(
                StatusTone::Error,
                "CATALOGUE UNAVAILABLE",
                "This shelf could not be loaded",
                error,
            ));
            section = section.child(
                div()
                    .id(format!("retry-home-{}", key.as_str()))
                    .tab_index(tab_start + 40)
                    .rounded_md()
                    .bg(rgb(Theme::ACCENT_DARK))
                    .px_3()
                    .py_2()
                    .cursor_pointer()
                    .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                    .aria_label(format!("Retry {title} catalogue"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.refresh_home_catalogue(cx);
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

        if let Some(error) = &shelf.load_more_error {
            section = section.child(status_panel(
                StatusTone::Error,
                "MORE TITLES UNAVAILABLE",
                "This shelf could not load the next page",
                error,
            ));
        }

        if shelf.can_load_more() || shelf.loading_more {
            let button_label = if shelf.loading_more {
                "Loading more…"
            } else if shelf.load_more_error.is_some() {
                "Retry"
            } else {
                "Load more"
            };
            let button_aria = if shelf.load_more_error.is_some() {
                format!("Retry loading more from {title}")
            } else {
                format!("Load more from {title}")
            };
            section = section.child(
                div()
                    .id(format!("home-{}-load-more", key.as_str()))
                    .tab_index(tab_start + 41)
                    .cursor_pointer()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(Theme::BORDER_STRONG))
                    .bg(rgb(Theme::SURFACE_RAISED))
                    .px_3()
                    .py_2()
                    .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                    .aria_label(button_aria)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.load_more_home_shelf(key, cx);
                    }))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_PRIMARY))
                            .child(button_label),
                    ),
            );
        }

        section
    }

    fn render_home_artwork(&self, cx: &mut Context<Self>) -> gpui::Div {
        let mut content = div().w_full().flex().flex_col().gap_6();

        for (index, preference) in self.home_settings.sections.iter().enumerate() {
            if !preference.enabled {
                continue;
            }
            let (key, shelf) = match preference.key.as_str() {
                "trending" => (HomeShelfKey::Trending, &self.home_trending),
                "popular-movies" => (HomeShelfKey::PopularMovies, &self.home_popular_movies),
                "popular-tv" => (HomeShelfKey::PopularTv, &self.home_popular_tv),
                _ => continue,
            };
            content = content.child(self.render_home_shelf(
                key,
                preference.display_title(),
                shelf,
                100 + (index as isize * 50),
                cx,
            ));
        }

        content
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
        let featured = self.home_featured_items.get(self.home_featured_index);
        let featured_title = featured
            .map(|item| item.title.as_str())
            .unwrap_or("Your entertainment. Your space.");
        let featured_eyebrow = featured
            .map(|item| item.detail.as_str())
            .unwrap_or("NATIVE MEDIA CENTER");
        let featured_description = featured
            .and_then(|item| item.overview.as_deref())
            .filter(|overview| !overview.trim().is_empty())
            .unwrap_or(
                "Discover films and series from the catalogues you have enabled for your Home screen.",
            )
            .to_owned();
        let featured_backdrop = featured.map(|item| item.backdrop.clone());
        let mut hero = backdrop_surface(
            featured_backdrop,
            286.0,
            self.home_catalogue_status == HomeCatalogueStatus::Loading,
        )
        .child(
            div()
                .absolute()
                .inset_0()
                .bg(rgb(Theme::BACKGROUND).opacity(0.76)),
        );

        let mut hero_content = div()
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
                    .child(featured_eyebrow.to_owned()),
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
                    .child(featured_description),
            );

        if self.home_featured_items.len() > 1 {
            let count = self.home_featured_items.len();
            let indicator = format!(
                "FEATURED  {} / {}",
                self.home_featured_index.saturating_add(1),
                count
            );
            hero_content = hero_content.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id("home-featured-previous")
                            .tab_index(32)
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(Theme::BORDER_STRONG))
                            .bg(rgb(Theme::SURFACE_RAISED))
                            .px_3()
                            .py_2()
                            .cursor_pointer()
                            .focus_visible(|style| {
                                style.border_1().border_color(rgb(Theme::ACCENT))
                            })
                            .aria_label("Show previous featured title")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.step_home_featured(-1, cx);
                            }))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(Theme::TEXT_PRIMARY))
                                    .child("‹"),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child(indicator),
                    )
                    .child(
                        div()
                            .id("home-featured-next")
                            .tab_index(33)
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(Theme::BORDER_STRONG))
                            .bg(rgb(Theme::SURFACE_RAISED))
                            .px_3()
                            .py_2()
                            .cursor_pointer()
                            .focus_visible(|style| {
                                style.border_1().border_color(rgb(Theme::ACCENT))
                            })
                            .aria_label("Show next featured title")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.step_home_featured(1, cx);
                            }))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(Theme::TEXT_PRIMARY))
                                    .child("›"),
                            ),
                    ),
            );
        }

        let mut hero_actions = div().flex().gap_3().child(
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
        );
        hero_actions = hero_actions.child(
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
        );
        if let Some(featured) = featured {
            let featured_id = featured.id;
            let featured_type = featured.media_type;
            let featured_name = featured.title.clone();
            let featured_poster = featured.poster.clone();
            hero_actions = hero_actions.child(
                div()
                    .id("home-featured-more-info")
                    .tab_index(34)
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(Theme::BORDER_STRONG))
                    .bg(rgb(Theme::SURFACE_RAISED))
                    .px_4()
                    .py_3()
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(Theme::SURFACE_HOVER)))
                    .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                    .aria_label(format!("More information about {}", featured_name))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_home_card(
                            featured_id,
                            featured_type,
                            featured_name.clone(),
                            featured_poster.clone(),
                            cx,
                        );
                    }))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_PRIMARY))
                            .child("More info"),
                    ),
            );
        }
        hero_content = hero_content.child(hero_actions);
        hero = hero.child(hero_content);

        let refresh_label = if self.home_catalogue_status == HomeCatalogueStatus::Loading {
            "Refreshing…"
        } else {
            "Refresh catalogues"
        };
        let home_toolbar = div()
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div().flex_1().child(
                    div()
                        .text_xs()
                        .text_color(rgb(Theme::TEXT_MUTED))
                        .child("HOME  /  DISCOVER"),
                ),
            )
            .child(
                div()
                    .id("home-refresh-catalogues")
                    .tab_index(29)
                    .cursor_pointer()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(Theme::BORDER_STRONG))
                    .bg(rgb(Theme::SURFACE_RAISED))
                    .px_3()
                    .py_2()
                    .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                    .aria_label("Refresh Home catalogues")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.refresh_home_catalogue(cx);
                    }))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_PRIMARY))
                            .child(refresh_label),
                    ),
            );

        let mut page = div().w_full().flex().flex_col().gap_6().child(home_toolbar);
        if self.home_settings.hero_enabled {
            page = page.child(hero);
        }
        page.child(self.render_home_artwork(cx)).child(
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
            .child(self.render_home_layout_settings(cx))
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
        if self.details.is_some() {
            return self.render_details(cx);
        }
        if self.person_details.is_some() {
            return self.render_person_details(cx);
        }
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
        self.details_cancellation.cancel();
        self.person_details_cancellation.cancel();
    }
}

impl Render for SenzoShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.smoke_test
            && self.navigation.current() == AppPage::Home
            && self.home_catalogue_status == HomeCatalogueStatus::NotRequested
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

pub fn run(
    tokio: Handle,
    smoke_test: bool,
    metadata: MetadataClient,
    artwork: ArtworkService,
    settings: Arc<dyn SettingsRepository>,
) {
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
                let shell = cx.new(|cx| {
                    SenzoShell::new(tokio, smoke_test, metadata, artwork, settings, window, cx)
                });
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
    use senzo_domain::{RepositoryError, SettingKey};

    struct TestSettingsRepository;

    impl SettingsRepository for TestSettingsRepository {
        fn get(&self, _key: &SettingKey) -> Result<Option<String>, RepositoryError> {
            Ok(None)
        }

        fn set(&self, _key: &SettingKey, _value: &str) -> Result<(), RepositoryError> {
            Ok(())
        }
    }

    #[test]
    fn search_filters_map_to_the_hosted_metadata_contract() {
        assert_eq!(SearchFilter::All.search_type(), SearchType::Multi);
        assert_eq!(SearchFilter::Movies.search_type(), SearchType::Movie);
        assert_eq!(SearchFilter::TvShows.search_type(), SearchType::Tv);
    }

    #[test]
    fn person_credit_selection_prefers_crew_for_non_acting_departments() {
        let person = Person {
            id: 42,
            name: "Filmmaker".to_owned(),
            biography: None,
            birthday: None,
            deathday: None,
            place_of_birth: None,
            profile_path: None,
            known_for_department: Some("Directing".to_owned()),
            also_known_as: None,
            gender: None,
            homepage: None,
            imdb_id: None,
            popularity: None,
            combined_credits: Some(senzo_metadata::PersonCredits {
                cast: vec![PersonCredit {
                    id: 10,
                    media_type: Some(MediaType::Movie),
                    title: Some("On Screen".to_owned()),
                    name: None,
                    original_title: None,
                    original_name: None,
                    overview: None,
                    poster_path: Some("/cast.jpg".to_owned()),
                    backdrop_path: None,
                    release_date: Some("2020-01-01".to_owned()),
                    first_air_date: None,
                    vote_average: Some(7.0),
                    vote_count: None,
                    popularity: Some(90.0),
                    character: Some("Himself".to_owned()),
                    job: None,
                    department: None,
                    episode_count: None,
                }],
                crew: vec![PersonCredit {
                    id: 11,
                    media_type: Some(MediaType::Movie),
                    title: Some("Directed Film".to_owned()),
                    name: None,
                    original_title: None,
                    original_name: None,
                    overview: None,
                    poster_path: Some("/crew.jpg".to_owned()),
                    backdrop_path: None,
                    release_date: Some("2021-01-01".to_owned()),
                    first_air_date: None,
                    vote_average: Some(8.0),
                    vote_count: None,
                    popularity: Some(10.0),
                    character: None,
                    job: Some("Director".to_owned()),
                    department: Some("Directing".to_owned()),
                    episode_count: None,
                }],
            }),
        };
        let credits = person_credit_candidates(&person, MediaType::Movie);
        assert_eq!(credits.len(), 1);
        assert_eq!(credits[0].title, "Directed Film");
        assert_eq!(credits[0].detail, "MOVIE · 2021 · Director");
    }

    #[test]
    fn person_credit_selection_uses_cast_for_actors_and_deduplicates() {
        let person = Person {
            id: 42,
            name: "Actor".to_owned(),
            biography: None,
            birthday: None,
            deathday: None,
            place_of_birth: None,
            profile_path: None,
            known_for_department: Some("Acting".to_owned()),
            also_known_as: None,
            gender: None,
            homepage: None,
            imdb_id: None,
            popularity: None,
            combined_credits: Some(senzo_metadata::PersonCredits {
                cast: vec![
                    PersonCredit {
                        id: 10,
                        media_type: Some(MediaType::Movie),
                        title: Some("Popular Film".to_owned()),
                        name: None,
                        original_title: None,
                        original_name: None,
                        overview: None,
                        poster_path: Some("/film.jpg".to_owned()),
                        backdrop_path: None,
                        release_date: Some("2022-01-01".to_owned()),
                        first_air_date: None,
                        vote_average: None,
                        vote_count: None,
                        popularity: Some(80.0),
                        character: Some("Lead".to_owned()),
                        job: None,
                        department: None,
                        episode_count: None,
                    },
                    PersonCredit {
                        id: 10,
                        media_type: Some(MediaType::Movie),
                        title: Some("Duplicate Film".to_owned()),
                        name: None,
                        original_title: None,
                        original_name: None,
                        overview: None,
                        poster_path: Some("/duplicate.jpg".to_owned()),
                        backdrop_path: None,
                        release_date: None,
                        first_air_date: None,
                        vote_average: None,
                        vote_count: None,
                        popularity: Some(70.0),
                        character: Some("Other".to_owned()),
                        job: None,
                        department: None,
                        episode_count: None,
                    },
                ],
                crew: Vec::new(),
            }),
        };
        let credits = person_credit_candidates(&person, MediaType::Movie);
        assert_eq!(credits.len(), 1);
        assert_eq!(credits[0].title, "Popular Film");
    }

    #[test]
    fn home_catalogue_candidates_support_movies_tv_and_people() {
        let candidates = home_media_candidates(
            vec![
                MediaItem {
                    id: 1,
                    media_type: None,
                    title: Some("Popular film".to_owned()),
                    name: None,
                    original_title: None,
                    original_name: None,
                    overview: None,
                    poster_path: Some("/film.jpg".to_owned()),
                    backdrop_path: None,
                    profile_path: None,
                    release_date: Some("2024-01-01".to_owned()),
                    first_air_date: None,
                    vote_average: None,
                    vote_count: None,
                    popularity: None,
                    adult: None,
                    original_language: None,
                },
                MediaItem {
                    id: 2,
                    media_type: Some(MediaType::Person),
                    title: None,
                    name: Some("Featured person".to_owned()),
                    original_title: None,
                    original_name: None,
                    overview: None,
                    poster_path: None,
                    backdrop_path: None,
                    profile_path: Some("/person.jpg".to_owned()),
                    release_date: None,
                    first_air_date: None,
                    vote_average: None,
                    vote_count: None,
                    popularity: None,
                    adult: None,
                    original_language: None,
                },
            ],
            Some(MediaType::Movie),
        );

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].media_type, MediaType::Movie);
        assert_eq!(candidates[0].detail, "MOVIE · 2024");
        assert_eq!(candidates[1].media_type, MediaType::Person);
        assert_eq!(candidates[1].poster_path, "/person.jpg");
    }

    #[test]
    fn built_in_home_pagination_caps_reported_pages_and_uses_the_requested_page() {
        assert_eq!(bounded_home_total_pages(1, Some(1)), 1);
        assert_eq!(bounded_home_total_pages(3, Some(2)), 3);
        assert_eq!(
            bounded_home_total_pages(3, Some(900)),
            MAX_HOME_CATALOGUE_PAGES
        );
        assert_eq!(bounded_home_total_pages(1, None), 1);
    }

    #[test]
    fn home_page_merge_deduplicates_by_media_identity_and_preserves_order() {
        let card = |id, media_type| HomeMediaCard {
            id,
            media_type,
            title: format!("Title {id}"),
            detail: "CATALOGUE".to_owned(),
            poster: None,
        };
        let mut existing = vec![card(10, MediaType::Movie)];
        let added = append_unique_home_cards(
            &mut existing,
            vec![
                card(10, MediaType::Movie),
                card(10, MediaType::Tv),
                card(11, MediaType::Movie),
            ],
        );

        assert_eq!(added, 2);
        assert_eq!(existing.len(), 3);
        assert_eq!(existing[0].media_type, MediaType::Movie);
        assert_eq!(existing[1].media_type, MediaType::Tv);
        assert_eq!(existing[2].id, 11);
    }

    #[test]
    fn home_hero_sources_follow_enabled_preferences_and_saved_order() {
        let mut settings = HomeSettings::default();
        assert_eq!(
            selected_home_hero_sources(&settings),
            vec![HomeShelfKey::Trending]
        );

        assert!(settings.set_hero_source_enabled("popular-movies", true));
        assert!(settings.move_section("popular-movies", -1));
        assert_eq!(
            selected_home_hero_sources(&settings),
            vec![HomeShelfKey::PopularMovies, HomeShelfKey::Trending]
        );

        assert!(settings.set_section_enabled("popular-movies", false));
        assert_eq!(
            selected_home_hero_sources(&settings),
            vec![HomeShelfKey::Trending]
        );
    }

    #[test]
    fn featured_carousel_navigation_wraps_in_both_directions() {
        assert_eq!(next_home_featured_index(0, 3, -1), 2);
        assert_eq!(next_home_featured_index(2, 3, 1), 0);
        assert_eq!(next_home_featured_index(1, 3, 0), 1);
        assert_eq!(next_home_featured_index(0, 0, 1), 0);
    }

    #[test]
    fn metadata_failures_have_actionable_user_messages() {
        assert!(
            metadata_error_message(&MetadataError::Network("timeout".to_owned()))
                .contains("internet connection")
        );
        assert!(
            metadata_error_message(&MetadataError::Api {
                status: 429,
                message: "rate limited".to_owned(),
            })
            .contains("Wait a moment")
        );
        assert!(
            metadata_error_message(&MetadataError::Api {
                status: 503,
                message: "upstream unavailable".to_owned(),
            })
            .contains("temporarily unavailable")
        );
        assert!(
            metadata_error_message(&MetadataError::Api {
                status: 404,
                message: "not found".to_owned(),
            })
            .contains("could not be found")
        );
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
                Arc::new(TestSettingsRepository),
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
