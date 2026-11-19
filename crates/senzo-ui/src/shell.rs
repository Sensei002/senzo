//! Native SENZO desktop shell: routes, focus, menus, dialogs and window actions.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use gpui::{
    App, Bounds, ColorExt, Context, Entity, FocusHandle, Image, ImageFormat, KeyBinding, Menu,
    MenuItem, MouseButton, NavigationDirection, QuitMode, Render, RenderImage, ScrollHandle,
    SharedString, Subscription, TitlebarOptions, Window, WindowBounds, WindowOptions, actions, div,
    img, point, prelude::*, px, rgb, size,
};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{
    input::{Input, InputEvent, InputState},
    slider::{Slider, SliderEvent, SliderState},
};
use senzo_addons::{
    AddonCatalogItem, AddonManager, AddonPreview, AddonStream, AddonSubtitle, ManagedAddon,
};
use senzo_artwork::{ArtworkService, ArtworkSize, ArtworkUrlResolver};
use senzo_catalog::{
    BuiltInHomeCatalog as HomeShelfKey, HomeCatalogDefinition, HomeCatalogProviderRegistry,
    HomeCollection, HomeCollectionsSettings, HomeSettings, home_collection_folder_key,
    visible_home_collections,
};
use senzo_debrid::{DebridManager, DebridProviderId, ResolvedDebridSource};
use senzo_domain::{
    AppPage, MediaKind, PlaybackProgress, PlaybackProgressRepository, SettingKey,
    SettingsRepository, WatchedItem, playback_is_complete, should_store_playback_progress,
};
use senzo_integrations::{MediaServerManager, MediaServerProvider};
#[cfg(test)]
use senzo_metadata::PersonCredit;
use senzo_metadata::{
    DiscoverRequest, DiscoverSort, EpisodeDetails, Genre, MediaItem, MediaType, MetadataClient,
    MetadataError, Person, SearchType, SeasonDetails, SeasonSummary, VideoItem,
};
use senzo_player::{
    MpvConfig, MpvPlayer, PlaybackSnapshot, PlaybackStatus, ResizeMode, TrackKind,
};
use senzo_plugins::{PluginManager, PluginState, PluginStream, ScraperInput};
use senzo_sources::{
    Source, SourceAggregator, SourceDiscoveryResult, SourceRequest, SourceSort, SourceTarget,
    sort_sources,
};
use serde_json::Value;
use tokio::{runtime::Handle, sync::mpsc, task::JoinSet};
use tokio_util::sync::CancellationToken;

use crate::{
    artwork::{backdrop_surface, poster_card, render_image_from_artwork},
    components::{
        StatusTone, divider, feature_tile, home_collection_row, page_intro, status_panel,
    },
    discovery::{parse_search_history, record_search_history},
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
        PlayerTogglePlayback,
        PlayerSeekBack,
        PlayerSeekForward,
        PlayerVolumeDown,
        PlayerVolumeUp,
        PlayerToggleMute,
        PlayerToggleFullscreen,
        PlayerCycleResize,
        PlayerShowAudio,
        PlayerShowSubtitles,
        PlayerShowChapters,
        PlayerShowInfo,
        PlayerShowSources,
        PlayerRetry,
        PlayerNextSource,
        PlayerPreviousEpisode,
        PlayerNextEpisode,
        PlayerTogglePip,
        Quit
    ]
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlayerPanel {
    Audio,
    Subtitles,
    Chapters,
    StreamInfo,
    Sources,
}

#[derive(Clone)]
struct PlaybackSourceCandidate {
    url: String,
    headers: BTreeMap<String, String>,
    title: String,
    provider_id: String,
    provider_name: String,
}

#[derive(Clone)]
struct PlaybackContext {
    media_id: String,
    media_kind: MediaKind,
    title: String,
    season_number: Option<u32>,
    episode_number: Option<u32>,
    episode_title: Option<String>,
    still_path: Option<String>,
    last_persisted_position_ms: u64,
}

#[derive(Clone)]
struct HomeUpNextCard {
    parent_progress: PlaybackProgress,
    season_number: u32,
    episode_number: u32,
    episode_title: String,
    air_date: Option<String>,
    still: Option<Arc<RenderImage>>,
}

fn current_utc_date() -> String {
    let days = (SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() / 86_400) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += if month <= 2 { 1 } else { 0 };
    format!("{year:04}-{month:02}-{day:02}")
}

fn playback_progress_artwork_key(progress: &PlaybackProgress) -> String {
    format!(
        "{}:{}:{}",
        progress.media_id,
        progress.season_number.unwrap_or(0),
        progress.episode_number.unwrap_or(0)
    )
}

fn current_epoch_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

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

const MAX_HOME_FEATURED_ITEMS: usize = 8;
const MAX_HOME_SHELF_ITEMS: usize = 120;
const MAX_HOME_CATALOGUE_PAGES: u32 = 500;
const MAX_HOME_COLLECTION_ARTWORK_ITEMS: usize = 120;

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

#[derive(Clone)]
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
        .filter_map(|section| HomeShelfKey::from_key(&section.key))
        .take(2)
        .collect()
}

/// Only publish a later hero source after all earlier selected sources have
/// completed. This keeps the visible carousel in the user's saved priority
/// order even when network requests finish out of order.
fn ready_home_hero_sources(
    settings: &HomeSettings,
    pending: &HashSet<HomeShelfKey>,
) -> Vec<HomeShelfKey> {
    let mut ready = Vec::new();
    for source in selected_home_hero_sources(settings) {
        if pending.contains(&source) {
            break;
        }
        ready.push(source);
    }
    ready
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
    descriptor_identity: String,
    page: u32,
    total_pages: u32,
    cards: Vec<HomeMediaCard>,
}

struct HomeShelfInitialResult {
    key: HomeShelfKey,
    descriptor_identity: String,
    data: HomeShelfLoadData,
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

fn home_card_focus_id(key: HomeShelfKey, media_type: MediaType, id: u64) -> String {
    format!(
        "home-{}-{}-{}",
        key.as_str(),
        home_media_type_key(media_type),
        id
    )
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

#[derive(Clone)]
struct SearchMediaCard {
    id: u64,
    media_type: MediaType,
    title: String,
    detail: String,
    overview: Option<String>,
    vote_average: Option<f64>,
    poster: Option<Arc<RenderImage>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiscoveryStatus {
    Idle,
    Loading,
    Results,
    Empty,
    Failed,
}

struct DiscoveryPageState {
    genres: Vec<Genre>,
    selected_genre: Option<u64>,
    selected_year: Option<u32>,
    sort: DiscoverSort,
    cards: Vec<SearchMediaCard>,
    page: u32,
    total_pages: u32,
    status: DiscoveryStatus,
    loading_more: bool,
    error: Option<String>,
    cancellation: CancellationToken,
    generation: u64,
}

impl Default for DiscoveryPageState {
    fn default() -> Self {
        Self {
            genres: Vec::new(),
            selected_genre: None,
            selected_year: None,
            sort: DiscoverSort::Popularity,
            cards: Vec::new(),
            page: 0,
            total_pages: 1,
            status: DiscoveryStatus::Idle,
            loading_more: false,
            error: None,
            cancellation: CancellationToken::new(),
            generation: 0,
        }
    }
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

enum DetailRoute {
    Title {
        id: u64,
        media_type: MediaType,
        title: String,
    },
    Person {
        id: u64,
        name: String,
        profile: Option<Arc<RenderImage>>,
    },
}

struct LoadedTitleDetails {
    title: String,
    imdb_id: Option<String>,
    overview: Option<String>,
    tagline: Option<String>,
    release_date: Option<String>,
    runtime: Option<u32>,
    status: Option<String>,
    rating: Option<f64>,
    rating_count: Option<u64>,
    genres: Vec<String>,
    cast: Vec<(u64, String, String)>,
    crew: Vec<(u64, String, String)>,
    production_companies: Vec<String>,
    production_countries: Vec<String>,
    trailers: Vec<VideoItem>,
    related: Vec<SearchMediaCard>,
    seasons: Vec<SeasonSummary>,
    related_artwork_paths: Vec<(usize, String)>,
    poster_path: Option<String>,
    backdrop_path: Option<String>,
    poster: Option<Arc<RenderImage>>,
    backdrop: Option<Arc<RenderImage>>,
}

struct DetailsPage {
    id: u64,
    imdb_id: Option<String>,
    media_type: MediaType,
    title: String,
    overview: Option<String>,
    tagline: Option<String>,
    release_date: Option<String>,
    runtime: Option<u32>,
    status: Option<String>,
    rating: Option<f64>,
    genres: Vec<String>,
    cast: Vec<(u64, String, String)>,
    crew: Vec<(u64, String, String)>,
    production_companies: Vec<String>,
    production_countries: Vec<String>,
    trailers: Vec<VideoItem>,
    related: Vec<SearchMediaCard>,
    rating_count: Option<u64>,
    seasons: Vec<SeasonSummary>,
    selected_season: Option<SeasonDetails>,
    selected_season_number: Option<u32>,
    season_loading: bool,
    season_error: Option<String>,
    season_cancellation: CancellationToken,
    selected_season_poster: Option<Arc<RenderImage>>,
    selected_episode: Option<EpisodeDetails>,
    selected_episode_identity: Option<(u64, u32, u32)>,
    selected_episode_artwork: Option<Arc<RenderImage>>,
    episode_loading: bool,
    episode_error: Option<String>,
    episode_cancellation: CancellationToken,
    poster: Option<Arc<RenderImage>>,
    backdrop: Option<Arc<RenderImage>>,
    loading: bool,
    error: Option<String>,
    source_result: Option<SourceDiscoveryResult>,
    source_loading: bool,
    source_error: Option<String>,
    source_season: Option<u32>,
    source_episode: Option<u32>,
    source_sort: SourceSort,
    source_show_direct: bool,
    source_show_torrents: bool,
    source_show_external: bool,
    resolved_debrid_source: Option<ResolvedDebridSource>,
    resolver_source_title: Option<String>,
    resolver_loading: bool,
    resolver_error: Option<String>,
}

pub struct ShellServices {
    pub tokio: Handle,
    pub metadata: MetadataClient,
    pub artwork: ArtworkService,
    pub settings: Arc<dyn SettingsRepository>,
    pub playback_progress: Arc<dyn PlaybackProgressRepository>,
    pub addon_manager: AddonManager,
    pub plugin_manager: PluginManager,
    pub debrid_manager: DebridManager,
    pub media_server_manager: MediaServerManager,
    pub source_aggregator: SourceAggregator,
}

pub(crate) struct SenzoShell {
    navigation: NavigationStack,
    navigation_focus: HashMap<AppPage, FocusHandle>,
    dialog_focus: FocusHandle,
    tokio: Handle,
    playback_progress_repository: Arc<dyn PlaybackProgressRepository>,
    home_continue_watching: Vec<PlaybackProgress>,
    home_continue_watching_artwork: HashMap<String, Arc<RenderImage>>,
    home_progress_artwork_requested: bool,
    home_up_next: Vec<HomeUpNextCard>,
    home_up_next_loading: bool,
    home_up_next_requested: bool,
    pending_continue_watching_episode: Option<(u64, u32, u32)>,
    addon_manager: AddonManager,
    addon_url_input: Entity<InputState>,
    addon_media_type_input: Entity<InputState>,
    addon_media_id_input: Entity<InputState>,
    addon_catalog_extras_input: Entity<InputState>,
    addon_state: Vec<ManagedAddon>,
    addon_registry_loaded: bool,
    addon_preview: Option<AddonPreview>,
    addon_allow_http: bool,
    addon_busy: bool,
    addon_error: Option<String>,
    addon_notice: Option<String>,
    addon_catalog_results: Vec<(String, String, Vec<AddonCatalogItem>)>,
    addon_meta_result: Option<Value>,
    addon_stream_results: Vec<AddonStream>,
    addon_subtitle_results: Vec<AddonSubtitle>,
    addon_result_message: Option<String>,
    plugin_manager: PluginManager,
    debrid_manager: DebridManager,
    media_server_manager: MediaServerManager,
    source_aggregator: SourceAggregator,
    mpv_player: Option<MpvPlayer>,
    playback_start_cancel: CancellationToken,
    playback_refresh_cancel: CancellationToken,
    playback_controls_timer: CancellationToken,
    playback_loading: bool,
    playback_error: Option<String>,
    playback_open: bool,
    playback_controls_visible: bool,
    playback_panel: Option<PlayerPanel>,
    playback_sources: Vec<PlaybackSourceCandidate>,
    playback_source_index: usize,
    playback_context: Option<PlaybackContext>,
    playback_resume_prompt: Option<PlaybackProgress>,
    playback_resume_source: Option<PlaybackSourceCandidate>,
    playback_resume_position_seconds: Option<f64>,
    playback_preferred_provider_id: Option<String>,
    playback_autoplay_next: bool,
    playback_end_handled: bool,
    autoplay_next_pending: bool,
    autoplay_next_target: Option<(u64, u32, u32)>,
    playback_resize_mode: ResizeMode,
    playback_fullscreen: bool,
    playback_picture_in_picture: bool,
    playback_subtitles_visible: bool,
    playback_subtitle_font_size: u32,
    playback_subtitle_delay_seconds: f64,
    playback_scrubbing: bool,
    subtitle_url_input: Entity<InputState>,
    playback_timeline: Entity<SliderState>,
    _playback_timeline_subscription: Subscription,
    debrid_provider_input: Entity<InputState>,
    debrid_api_key_input: Entity<InputState>,
    server_provider_input: Entity<InputState>,
    server_name_input: Entity<InputState>,
    server_address_input: Entity<InputState>,
    server_username_input: Entity<InputState>,
    server_password_input: Entity<InputState>,
    source_settings_busy: bool,
    source_settings_error: Option<String>,
    source_settings_notice: Option<String>,
    plugin_repository_input: Entity<InputState>,
    plugin_settings_input: Entity<InputState>,
    plugin_tmdb_id_input: Entity<InputState>,
    plugin_media_type_input: Entity<InputState>,
    plugin_season_input: Entity<InputState>,
    plugin_episode_input: Entity<InputState>,
    plugin_state: PluginState,
    plugin_registry_loaded: bool,
    plugin_busy: bool,
    plugin_error: Option<String>,
    plugin_notice: Option<String>,
    plugin_settings_layout: Option<String>,
    plugin_settings_preview: Option<String>,
    plugin_selected_scraper_id: Option<String>,
    plugin_stream_results: Vec<PluginStream>,
    metadata: MetadataClient,
    artwork: ArtworkService,
    settings: Arc<dyn SettingsRepository>,
    home_settings: HomeSettings,
    home_collections: Vec<HomeCollection>,
    home_collection_artwork: HashMap<String, Arc<RenderImage>>,
    home_collection_artwork_requested: bool,
    home_collection_artwork_cancellation: CancellationToken,
    home_cancellation: CancellationToken,
    home_catalogue_generation: u64,
    home_catalogue_status: HomeCatalogueStatus,
    home_catalogue_descriptor_identities: HashMap<HomeShelfKey, String>,
    home_trending: HomeShelfState,
    home_popular_movies: HomeShelfState,
    home_popular_tv: HomeShelfState,
    home_featured_items: Vec<HomeFeaturedCard>,
    home_featured_index: usize,
    home_featured_by_source: HashMap<HomeShelfKey, Vec<HomeFeaturedCard>>,
    home_featured_sources_pending: HashSet<HomeShelfKey>,
    home_title_inputs: HashMap<String, Entity<InputState>>,
    _home_title_subscriptions: Vec<Subscription>,
    home_title_errors: HashMap<String, String>,
    home_card_focus_handles: HashMap<String, FocusHandle>,
    last_home_focus_id: Option<String>,
    home_return_focus_id: Option<String>,
    page_scroll_handle: ScrollHandle,
    page_scroll_offsets: HashMap<AppPage, gpui::Point<gpui::Pixels>>,
    search_input: Entity<InputState>,
    _search_input_subscription: Subscription,
    search_filter: SearchFilter,
    search_query: String,
    recent_searches: Vec<String>,
    search_page: u32,
    search_total_pages: u32,
    search_status: SearchStatus,
    search_error: Option<String>,
    search_results: Vec<SearchMediaCard>,
    search_total_results: Option<u32>,
    selected_search_result: Option<usize>,
    search_artwork_loading: bool,
    search_cancellation: CancellationToken,
    search_generation: u64,
    movie_discovery: DiscoveryPageState,
    tv_discovery: DiscoveryPageState,
    details: Option<DetailsPage>,
    details_cancellation: CancellationToken,
    person_details: Option<PersonDetailsPage>,
    person_details_cancellation: CancellationToken,
    detail_history: Vec<DetailRoute>,
    logo: Arc<Image>,
    dialog: Option<DialogKind>,
    menu_open: bool,
    notification: Option<String>,
    runtime_status: String,
    smoke_test: bool,
    smoke_test_scheduled: bool,
}

fn parse_addon_extras(raw: &str) -> Result<BTreeMap<String, String>, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut extras = BTreeMap::new();
    for pair in raw.split('&') {
        let (key, value) = pair
            .split_once('=')
            .ok_or_else(|| "Each catalogue extra must use key=value.".to_owned())?;
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() || value.is_empty() {
            return Err("Catalogue extra keys and values cannot be empty.".to_owned());
        }
        if extras.insert(key.to_owned(), value.to_owned()).is_some() {
            return Err(format!("Catalogue extra '{key}' is duplicated."));
        }
    }
    Ok(extras)
}

fn format_playback_time(seconds: f64) -> String {
    let total = if seconds.is_finite() {
        seconds.max(0.0) as u64
    } else {
        0
    };
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let remaining = total % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{remaining:02}")
    } else {
        format!("{minutes}:{remaining:02}")
    }
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

fn chapter_skip_target(snapshot: &PlaybackSnapshot) -> Option<(String, f64)> {
    let position = snapshot.position_seconds.unwrap_or_default();
    if !position.is_finite() || position < 0.0 || snapshot.chapters.len() < 2 {
        return None;
    }
    let current_index = snapshot.chapters.iter().enumerate()
        .filter(|(_, chapter)| chapter.start_seconds <= position)
        .map(|(index, _)| index)
        .last()?;
    let chapter = snapshot.chapters.get(current_index)?;
    let title = chapter.title.as_deref()?.trim().to_ascii_lowercase();
    let label = if title.contains("recap") {
        "SKIP RECAP"
    } else if title.contains("intro") || title.contains("opening credits") || title.contains("opening") {
        "SKIP INTRO"
    } else if title.contains("credits") || title.contains("outro") || title.contains("ending") {
        "SKIP CREDITS"
    } else {
        return None;
    };
    let target = snapshot.chapters.get(current_index + 1)
        .map(|next| next.start_seconds)
        .or(snapshot.duration_seconds)?;
    (target > position).then_some((label.to_owned(), target))
}

impl SenzoShell {
    fn new(
        services: ShellServices,
        smoke_test: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let ShellServices {
            tokio,
            metadata,
            artwork,
            settings,
            playback_progress,
            addon_manager,
            plugin_manager,
            debrid_manager,
            media_server_manager,
            source_aggregator,
        } = services;
        let navigation_focus = AppPage::ALL
            .iter()
            .enumerate()
            .map(|(index, page)| (*page, cx.focus_handle().tab_index(index as isize)))
            .collect();
        let dialog_focus = cx.focus_handle().tab_index(60);
        let search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search movies, TV shows, and people")
        });
        let subtitle_url_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("https://subtitles.example/subtitle.srt")
        });
        let playback_timeline = cx.new(|_| SliderState::new());
        let playback_timeline_subscription =
            cx.subscribe(&playback_timeline, |this, _state, event, cx| {
                if let SliderEvent::Change(_) = event {
                    this.playback_scrubbing = true;
                    return;
                }
                if let SliderEvent::Release(value) = event {
                    this.playback_scrubbing = false;
                    let Some(duration) = this
                        .mpv_player
                        .as_ref()
                        .and_then(|player| player.snapshot().duration_seconds)
                    else {
                        return;
                    };
                    let target = duration * (value.end().clamp(0.0, 100.0) as f64 / 100.0);
                    this.seek_playback_absolute(target, cx);
                }
            });
        let addon_url_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("https://provider.example/manifest.json")
        });
        let addon_media_type_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value("movie")
                .placeholder("movie or series")
        });
        let addon_media_id_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("IMDb ID, e.g. tt0133093"));
        let addon_catalog_extras_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Optional catalogue extras: genre=action&skip=0")
        });
        let plugin_repository_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("https://plugins.example/repository")
        });
        let plugin_settings_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value("{}")
                .placeholder("Plugin settings JSON")
        });
        let plugin_tmdb_id_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value("603")
                .placeholder("TMDB ID")
        });
        let plugin_media_type_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value("movie")
                .placeholder("movie or tv")
        });
        let plugin_season_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Season (TV only)"));
        let plugin_episode_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Episode (TV only)"));
        let debrid_provider_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value("torbox")
                .placeholder("torbox, premiumize, or realdebrid")
        });
        let debrid_api_key_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Debrid API key"));
        let server_provider_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value("jellyfin")
                .placeholder("jellyfin or emby")
        });
        let server_name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value("My media server")
                .placeholder("Display name")
        });
        let server_address_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("http://192.168.1.20:8096"));
        let server_username_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Username"));
        let server_password_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Password"));
        let search_input_subscription = cx.subscribe(&search_input, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.update_search_query(input.read(cx).value().to_string(), cx);
            }
        });
        let home_continue_watching = playback_progress
            .list_continue_watching("default", 20)
            .unwrap_or_else(|error| {
                tracing::warn!(error = %error, "could not load Continue Watching progress");
                Vec::new()
            });
        let autoplay_setting_key =
            SettingKey::new("playback.autoplay_next").expect("valid autoplay setting key");
        let playback_autoplay_next = settings
            .get(&autoplay_setting_key)
            .ok()
            .flatten()
            .map(|value| value == "true")
            .unwrap_or(true);
        let search_history_key =
            SettingKey::new("search.recent").expect("valid search history key");
        let search_history_payload = settings.get(&search_history_key).unwrap_or_else(|error| {
            tracing::warn!(error = %error, "could not load search history");
            None
        });
        let recent_searches = parse_search_history(search_history_payload.as_deref());
        let home_settings = HomeSettings::load(settings.as_ref()).unwrap_or_else(|error| {
            tracing::warn!(error = %error, "could not load persisted Home layout; using defaults");
            HomeSettings::default()
        });
        let home_collections = HomeCollectionsSettings::load(settings.as_ref())
            .unwrap_or_else(|error| {
                tracing::warn!(error = %error, "could not load Home collections; using an empty collection list");
                HomeCollectionsSettings::default()
            })
            .collections;
        let home_featured_info_focus = cx.focus_handle();
        let mut home_card_focus_handles = HashMap::new();
        home_card_focus_handles.insert(
            "home-featured-more-info".to_owned(),
            home_featured_info_focus,
        );
        let mut home_title_inputs = HashMap::new();
        let mut home_title_subscriptions = Vec::new();
        for section in &home_settings.sections {
            let key = section.key.clone();
            let initial_title = section.title.clone().unwrap_or_default();
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(initial_title)
                    .placeholder("Custom title (optional)")
            });
            let input_key = key.clone();
            let subscription = cx.subscribe(&input, move |this, input, event, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.save_home_section_title(
                        &input_key,
                        input.read(cx).value().to_string(),
                        cx,
                    );
                }
            });
            home_title_inputs.insert(key, input);
            home_title_subscriptions.push(subscription);
        }

        Self {
            navigation: NavigationStack::new(),
            navigation_focus,
            dialog_focus,
            tokio,
            playback_progress_repository: playback_progress,
            home_continue_watching,
            home_continue_watching_artwork: HashMap::new(),
            home_progress_artwork_requested: false,
            home_up_next: Vec::new(),
            home_up_next_loading: false,
            home_up_next_requested: false,
            pending_continue_watching_episode: None,
            addon_manager,
            addon_url_input,
            addon_media_type_input,
            addon_media_id_input,
            addon_catalog_extras_input,
            addon_state: Vec::new(),
            addon_registry_loaded: false,
            addon_preview: None,
            addon_allow_http: false,
            addon_busy: false,
            addon_error: None,
            addon_notice: None,
            addon_catalog_results: Vec::new(),
            addon_meta_result: None,
            addon_stream_results: Vec::new(),
            addon_subtitle_results: Vec::new(),
            addon_result_message: None,
            plugin_manager,
            debrid_manager,
            media_server_manager,
            source_aggregator,
            mpv_player: None,
            playback_start_cancel: CancellationToken::new(),
            playback_refresh_cancel: CancellationToken::new(),
            playback_controls_timer: CancellationToken::new(),
            playback_loading: false,
            playback_error: None,
            playback_open: false,
            playback_controls_visible: true,
            playback_panel: None,
            playback_sources: Vec::new(),
            playback_source_index: 0,
            playback_context: None,
            playback_resume_prompt: None,
            playback_resume_source: None,
            playback_resume_position_seconds: None,
            playback_preferred_provider_id: None,
            playback_autoplay_next,
            playback_end_handled: false,
            autoplay_next_pending: false,
            autoplay_next_target: None,
            playback_resize_mode: ResizeMode::Fit,
            playback_fullscreen: false,
            playback_picture_in_picture: false,
            playback_subtitles_visible: true,
            playback_subtitle_font_size: 36,
            playback_subtitle_delay_seconds: 0.0,
            playback_scrubbing: false,
            subtitle_url_input,
            playback_timeline,
            _playback_timeline_subscription: playback_timeline_subscription,
            plugin_repository_input,
            plugin_settings_input,
            plugin_tmdb_id_input,
            plugin_media_type_input,
            plugin_season_input,
            plugin_episode_input,
            debrid_provider_input,
            debrid_api_key_input,
            server_provider_input,
            server_name_input,
            server_address_input,
            server_username_input,
            server_password_input,
            source_settings_busy: false,
            source_settings_error: None,
            source_settings_notice: None,
            plugin_state: PluginState::default(),
            plugin_registry_loaded: false,
            plugin_busy: false,
            plugin_error: None,
            plugin_notice: None,
            plugin_settings_layout: None,
            plugin_settings_preview: None,
            plugin_selected_scraper_id: None,
            plugin_stream_results: Vec::new(),
            metadata,
            artwork,
            settings,
            home_settings,
            home_collections,
            home_collection_artwork: HashMap::new(),
            home_collection_artwork_requested: false,
            home_collection_artwork_cancellation: CancellationToken::new(),
            home_cancellation: CancellationToken::new(),
            home_catalogue_generation: 0,
            home_catalogue_status: HomeCatalogueStatus::NotRequested,
            home_catalogue_descriptor_identities: HashMap::new(),
            home_trending: HomeShelfState::default(),
            home_popular_movies: HomeShelfState::default(),
            home_popular_tv: HomeShelfState::default(),
            home_featured_items: Vec::new(),
            home_featured_index: 0,
            home_featured_by_source: HashMap::new(),
            home_featured_sources_pending: HashSet::new(),
            home_title_inputs,
            _home_title_subscriptions: home_title_subscriptions,
            home_title_errors: HashMap::new(),
            home_card_focus_handles,
            last_home_focus_id: None,
            home_return_focus_id: None,
            page_scroll_handle: ScrollHandle::new(),
            page_scroll_offsets: HashMap::new(),
            search_input,
            _search_input_subscription: search_input_subscription,
            search_filter: SearchFilter::All,
            search_query: String::new(),
            recent_searches,
            search_page: 1,
            search_total_pages: 1,
            search_status: SearchStatus::Idle,
            search_error: None,
            search_results: Vec::new(),
            search_total_results: None,
            selected_search_result: None,
            search_artwork_loading: false,
            search_cancellation: CancellationToken::new(),
            search_generation: 0,
            movie_discovery: DiscoveryPageState::default(),
            tv_discovery: DiscoveryPageState::default(),
            details: None,
            details_cancellation: CancellationToken::new(),
            person_details: None,
            person_details_cancellation: CancellationToken::new(),
            detail_history: Vec::new(),
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

    fn remember_current_page_scroll(&mut self) {
        self.page_scroll_offsets
            .insert(self.navigation.current(), self.page_scroll_handle.offset());
    }

    fn focused_home_card_id(&self, window: &Window) -> Option<String> {
        self.home_card_focus_handles
            .iter()
            .find(|(_, handle)| handle.is_focused(window))
            .map(|(id, _)| id.clone())
    }

    fn remember_focused_home_card(&mut self, window: &Window) {
        if self.navigation.current() == AppPage::Home
            && let Some(id) = self.focused_home_card_id(window)
        {
            self.last_home_focus_id = Some(id);
        }
    }

    fn ensure_home_card_focus_handles(
        &mut self,
        key: HomeShelfKey,
        cards: &[HomeMediaCard],
        start_index: usize,
        cx: &mut Context<Self>,
    ) {
        let section_index = self
            .home_settings
            .sections
            .iter()
            .position(|section| section.key == key.as_str())
            .unwrap_or(0);
        let base_tab = 100 + (section_index as isize * 50);
        for (index, card) in cards.iter().enumerate() {
            let id = home_card_focus_id(key, card.media_type, card.id);
            self.home_card_focus_handles.entry(id).or_insert_with(|| {
                cx.focus_handle()
                    .tab_index(base_tab + start_index as isize + index as isize)
            });
        }
    }

    fn restore_page_position(&self, page: AppPage, focus_id: Option<String>, window: &mut Window) {
        let offset = self
            .page_scroll_offsets
            .get(&page)
            .copied()
            .unwrap_or_else(|| point(px(0.0), px(0.0)));
        let scroll_handle = self.page_scroll_handle.clone();
        let focus_handle = focus_id
            .as_ref()
            .and_then(|id| self.home_card_focus_handles.get(id))
            .cloned();

        window.on_next_frame(move |window, cx| {
            scroll_handle.set_offset(offset);
            if let Some(focus_handle) = focus_handle {
                window.focus(&focus_handle, cx);
            }
        });
    }

    fn focus_page(&self, page: AppPage, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(handle) = self.navigation_focus.get(&page) {
            window.focus(handle, cx);
        }
    }

    fn navigate(&mut self, page: AppPage, window: &mut Window, cx: &mut Context<Self>) {
        self.stop_playback();
        self.remember_current_page_scroll();
        self.remember_focused_home_card(window);
        if self.navigation.navigate(page) {
            self.menu_open = false;
            if page == AppPage::Addons {
                self.load_addon_registry(cx);
            } else if page == AppPage::Settings {
                self.load_plugin_registry(cx);
            } else if page == AppPage::Movies {
                self.ensure_discovery_loaded(MediaType::Movie, cx);
            } else if page == AppPage::TvShows {
                self.ensure_discovery_loaded(MediaType::Tv, cx);
            }
            self.focus_page(page, window, cx);
            let focus_id = (page == AppPage::Home)
                .then(|| self.last_home_focus_id.clone())
                .flatten();
            self.restore_page_position(page, focus_id, window);
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
        if self.playback_open {
            self.close_player(window, cx);
            return;
        }
        if self.details.is_some() {
            self.close_details(window, cx);
            return;
        }
        if self.person_details.is_some() {
            self.close_person_details(window, cx);
            return;
        }
        self.remember_current_page_scroll();
        self.remember_focused_home_card(window);
        if self.navigation.back() {
            let page = self.navigation.current();
            if page == AppPage::Addons {
                self.load_addon_registry(cx);
            } else if page == AppPage::Settings {
                self.load_plugin_registry(cx);
            }
            self.focus_page(page, window, cx);
            let focus_id = (page == AppPage::Home)
                .then(|| self.last_home_focus_id.clone())
                .flatten();
            self.restore_page_position(page, focus_id, window);
            cx.notify();
        }
    }

    fn go_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.is_some() || self.menu_open {
            return;
        }
        self.remember_current_page_scroll();
        self.remember_focused_home_card(window);
        if self.navigation.can_go_forward() && self.navigation.forward() {
            let page = self.navigation.current();
            if page == AppPage::Addons {
                self.load_addon_registry(cx);
            } else if page == AppPage::Settings {
                self.load_plugin_registry(cx);
            }
            self.focus_page(page, window, cx);
            let focus_id = (page == AppPage::Home)
                .then(|| self.last_home_focus_id.clone())
                .flatten();
            self.restore_page_position(page, focus_id, window);
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

    fn save_home_section_title(&mut self, key: &str, raw_title: String, cx: &mut Context<Self>) {
        let trimmed = raw_title.trim();
        let current = self
            .home_settings
            .sections
            .iter()
            .find(|section| section.key == key)
            .and_then(|section| section.title.as_deref())
            .unwrap_or_default();
        if current == trimmed {
            self.home_title_errors.remove(key);
            return;
        }

        match self.home_settings.set_section_title(key, Some(trimmed)) {
            Ok(()) => {
                self.home_title_errors.remove(key);
                self.persist_home_settings(cx);
            }
            Err(error) => {
                self.home_title_errors
                    .insert(key.to_owned(), error.to_string());
                self.notification = Some(
                    "Custom titles must be 1–80 printable characters. Clear the field to restore the default title."
                        .to_owned(),
                );
                cx.notify();
            }
        }
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
        self.refresh_home_catalogue(cx);
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
        self.home_featured_by_source.clear();
        self.home_featured_sources_pending.clear();
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
        let Some(descriptor_identity) =
            self.home_catalogue_descriptor_identities.get(&key).cloned()
        else {
            self.home_shelf_mut(key).loading_more = false;
            cx.notify();
            return;
        };
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
                descriptor_identity,
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

                if let Ok(data) = &result
                    && this.home_catalogue_descriptor_identities.get(&key)
                        != Some(&data.descriptor_identity)
                {
                    this.home_shelf_mut(key).loading_more = false;
                    return;
                }

                if let Ok(data) = &result {
                    let start_index = this.home_shelf_mut(key).cards.len();
                    this.ensure_home_card_focus_handles(key, &data.cards, start_index, cx);
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
            let base_tab = 50 + (index as isize * 5);
            let title_input = self.home_title_inputs.get(&key);
            let title_error = self.home_title_errors.get(&key).cloned();

            let mut section_identity = div()
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
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(Theme::TEXT_MUTED))
                        .child(if enabled {
                            "Visible on Home"
                        } else {
                            "Hidden from Home"
                        }),
                );
            if let Some(input) = title_input {
                section_identity = section_identity.child(
                    Input::new(input)
                        .tab_index(base_tab + 4)
                        .aria_label(format!("Custom Home shelf title for {}", section.key))
                        .w_full()
                        .max_w(px(360.0))
                        .bg(rgb(Theme::SURFACE))
                        .border_1()
                        .border_color(rgb(Theme::BORDER))
                        .rounded_md()
                        .px_2()
                        .py_2(),
                );
            }
            if let Some(error) = title_error {
                section_identity = section_identity
                    .child(div().text_xs().text_color(rgb(Theme::DANGER)).child(error));
            }

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
                .child(section_identity)
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
    fn rebuild_home_featured_items(&mut self) {
        let current = self
            .home_featured_items
            .get(self.home_featured_index)
            .map(|item| (item.id, item.media_type));

        if !self.home_settings.hero_enabled {
            self.home_featured_items.clear();
            self.home_featured_index = 0;
            return;
        }

        let ready_sources =
            ready_home_hero_sources(&self.home_settings, &self.home_featured_sources_pending);
        let mut featured = Vec::with_capacity(MAX_HOME_FEATURED_ITEMS);
        for source in ready_sources {
            if let Some(items) = self.home_featured_by_source.get(&source) {
                for item in items {
                    if featured.len() >= MAX_HOME_FEATURED_ITEMS {
                        break;
                    }
                    featured.push(item.clone());
                }
            }
            if featured.len() >= MAX_HOME_FEATURED_ITEMS {
                break;
            }
        }

        self.home_featured_items = featured;
        self.home_featured_index = current
            .and_then(|(id, media_type)| {
                self.home_featured_items
                    .iter()
                    .position(|item| item.id == id && item.media_type == media_type)
            })
            .unwrap_or(0);
    }

    fn load_home_collection_artwork(&mut self, cx: &mut Context<Self>) {
        if self.home_collection_artwork_requested {
            return;
        }
        self.home_collection_artwork_requested = true;

        let requests = home_collection_artwork_requests(&self.home_collections);
        if requests.is_empty() {
            return;
        }

        let artwork = self.artwork.clone();
        let cancellation = self.home_collection_artwork_cancellation.child_token();
        let completion_cancellation = cancellation.clone();
        let (sender, mut receiver) = mpsc::unbounded_channel::<(String, Arc<RenderImage>)>();
        let task = self.tokio.spawn(async move {
            let mut jobs = JoinSet::new();
            for (key, path) in requests {
                let service = artwork.clone();
                let token = cancellation.child_token();
                jobs.spawn(async move {
                    let image = load_gpui_artwork(service, path, ArtworkSize::W342, token).await;
                    (key, image)
                });
            }

            while let Some(result) = jobs.join_next().await {
                if cancellation.is_cancelled() {
                    break;
                }
                if let Ok((key, Some(image))) = result
                    && sender.send((key, image)).is_err()
                {
                    break;
                }
            }
        });

        cx.spawn(async move |this, cx| {
            while let Some((key, image)) = receiver.recv().await {
                let _ = this.update(cx, |this, cx| {
                    if completion_cancellation.is_cancelled() {
                        return;
                    }
                    this.home_collection_artwork.insert(key, image);
                    cx.notify();
                });
            }
            let _ = task.await;
        })
        .detach();
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

        let definitions = HomeCatalogProviderRegistry::built_ins().definitions();
        self.home_catalogue_descriptor_identities = definitions
            .iter()
            .filter_map(|definition| {
                definition
                    .built_in_catalog()
                    .map(|catalog| (catalog, definition.cache_identity()))
            })
            .collect();

        let metadata = self.metadata.clone();
        let artwork = self.artwork.clone();
        let hero_sources = if self.home_settings.hero_enabled {
            selected_home_hero_sources(&self.home_settings)
        } else {
            Vec::new()
        };
        self.home_featured_by_source.clear();
        self.home_featured_sources_pending = hero_sources.iter().copied().collect();
        self.rebuild_home_featured_items();

        let cancellation = self.home_cancellation.child_token();
        let completion_cancellation = cancellation.clone();
        let (sender, mut receiver) = mpsc::unbounded_channel::<HomeShelfInitialResult>();
        let task = self.tokio.spawn(async move {
            let mut jobs = JoinSet::new();
            for definition in definitions {
                let Some(key) = definition.built_in_catalog() else {
                    continue;
                };
                let metadata = metadata.clone();
                let artwork = artwork.clone();
                let token = cancellation.child_token();
                let is_hero_source = hero_sources.contains(&key);
                jobs.spawn(async move {
                    load_home_shelf_initial(definition, metadata, artwork, is_hero_source, token)
                        .await
                });
            }

            while let Some(result) = jobs.join_next().await {
                if cancellation.is_cancelled() {
                    break;
                }
                if let Ok(Some(result)) = result
                    && sender.send(result).is_err()
                {
                    break;
                }
            }
        });

        cx.spawn(async move |this, cx| {
            while let Some(result) = receiver.recv().await {
                let _ = this.update(cx, |this, cx| {
                    if this.home_catalogue_generation != generation
                        || completion_cancellation.is_cancelled()
                    {
                        return;
                    }

                    let HomeShelfInitialResult {
                        key,
                        descriptor_identity,
                        data,
                        featured,
                    } = result;
                    if this.home_catalogue_descriptor_identities.get(&key)
                        != Some(&descriptor_identity)
                    {
                        return;
                    }
                    this.ensure_home_card_focus_handles(key, &data.cards, 0, cx);
                    *this.home_shelf_mut(key) = HomeShelfState {
                        status: HomeCatalogueStatus::Loaded,
                        error: data.error,
                        cards: data.cards,
                        page: data.page,
                        total_pages: data.total_pages,
                        loading_more: false,
                        load_more_error: None,
                    };

                    if this.home_featured_sources_pending.remove(&key) {
                        this.home_featured_by_source.insert(key, featured);
                        this.rebuild_home_featured_items();
                    }

                    if [
                        &this.home_trending,
                        &this.home_popular_movies,
                        &this.home_popular_tv,
                    ]
                    .iter()
                    .all(|shelf| shelf.status == HomeCatalogueStatus::Loaded)
                    {
                        this.home_catalogue_status = HomeCatalogueStatus::Loaded;
                    }
                    cx.notify();
                });
            }

            let worker_result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.home_catalogue_generation != generation
                    || completion_cancellation.is_cancelled()
                {
                    return;
                }

                let worker_stopped = worker_result.is_err();
                for key in [
                    HomeShelfKey::Trending,
                    HomeShelfKey::PopularMovies,
                    HomeShelfKey::PopularTv,
                ] {
                    let shelf = this.home_shelf_mut(key);
                    if shelf.status == HomeCatalogueStatus::Loading {
                        shelf.status = HomeCatalogueStatus::Loaded;
                        shelf.error = Some(if worker_stopped {
                            "A Home catalogue worker stopped unexpectedly. Refresh Home to retry."
                                .to_owned()
                        } else {
                            "This catalogue did not finish loading. Refresh Home to retry."
                                .to_owned()
                        });
                        shelf.cards.clear();
                        shelf.page = 0;
                        shelf.total_pages = 1;
                        shelf.loading_more = false;
                        shelf.load_more_error = None;
                    }
                    this.home_featured_sources_pending.remove(&key);
                }
                this.home_catalogue_status = HomeCatalogueStatus::Loaded;
                this.rebuild_home_featured_items();
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
        focus_id: String,
        cx: &mut Context<Self>,
    ) {
        self.remember_current_page_scroll();
        self.home_return_focus_id = Some(focus_id);
        self.last_home_focus_id = self.home_return_focus_id.clone();
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
        if !matches!(media_type, MediaType::Movie | MediaType::Tv) {
            return;
        }
        self.remember_current_page_scroll();
        self.person_details_cancellation.cancel();
        self.person_details = None;
        self.details_cancellation.cancel();
        let cancellation = CancellationToken::new();
        self.details_cancellation = cancellation.clone();
        self.details = Some(DetailsPage {
            id,
            imdb_id: None,
            media_type,
            title: initial_title,
            overview: None,
            tagline: None,
            release_date: None,
            runtime: None,
            status: None,
            rating: None,
            rating_count: None,
            genres: Vec::new(),
            cast: Vec::new(),
            crew: Vec::new(),
            production_companies: Vec::new(),
            production_countries: Vec::new(),
            trailers: Vec::new(),
            related: Vec::new(),
            seasons: Vec::new(),
            selected_season: None,
            selected_season_number: None,
            season_loading: false,
            season_error: None,
            season_cancellation: CancellationToken::new(),
            selected_season_poster: None,
            selected_episode: None,
            selected_episode_identity: None,
            selected_episode_artwork: None,
            episode_loading: false,
            episode_error: None,
            episode_cancellation: CancellationToken::new(),
            poster: None,
            backdrop: None,
            loading: true,
            error: None,
            source_result: None,
            source_loading: false,
            source_error: None,
            source_season: None,
            source_episode: None,
            source_sort: SourceSort::Recommended,
            source_show_direct: true,
            source_show_torrents: true,
            source_show_external: true,
            resolved_debrid_source: None,
            resolver_source_title: None,
            resolver_loading: false,
            resolver_error: None,
        });
        let metadata = self.metadata.clone();
        let artwork = self.artwork.clone();
        let task_token = cancellation.clone();
        let task = self.tokio.spawn(async move {
            let token = task_token.clone();
            let mut loaded = match media_type {
                MediaType::Movie => {
                    let movie = metadata
                        .movie(id, &token)
                        .await
                        .map_err(|error| metadata_error_message(&error))?;
                    let credits = movie.credits.as_ref();
                    let cast = credits
                        .map(|credits| {
                            credits
                                .cast
                                .iter()
                                .take(18)
                                .map(|person| {
                                    (
                                        person.id,
                                        person.name.clone(),
                                        person.character.clone().unwrap_or_else(|| "Cast".into()),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let crew = credits
                        .map(|credits| {
                            credits
                                .crew
                                .iter()
                                .filter(|person| {
                                    matches!(
                                        person.department.as_deref(),
                                        Some("Directing" | "Writing" | "Production")
                                    )
                                })
                                .take(12)
                                .map(|person| {
                                    (
                                        person.id,
                                        person.name.clone(),
                                        person
                                            .job
                                            .clone()
                                            .or(person.department.clone())
                                            .unwrap_or_else(|| "Crew".into()),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let related_source = movie
                        .recommendations
                        .as_ref()
                        .filter(|list| !list.results.is_empty())
                        .or(movie.similar.as_ref());
                    let mut related = Vec::new();
                    let mut related_artwork_paths = Vec::new();
                    if let Some(list) = related_source {
                        for item in list.results.iter().take(12) {
                            let kind = item.media_type.unwrap_or(MediaType::Movie);
                            if !matches!(kind, MediaType::Movie | MediaType::Tv) {
                                continue;
                            }
                            let detail = item
                                .release_year()
                                .map(|year| format!("MOVIE · {year}"))
                                .unwrap_or_else(|| "MOVIE".to_owned());
                            if let Some(path) = item.poster_path.clone() {
                                related_artwork_paths.push((related.len(), path));
                            }
                            related.push(SearchMediaCard {
                                id: item.id,
                                media_type: kind,
                                title: item.display_title().to_owned(),
                                detail,
                                overview: item.overview.clone(),
                                vote_average: item.vote_average,
                                poster: None,
                            });
                        }
                    }
                    let trailers = movie
                        .videos
                        .as_ref()
                        .map(|videos| {
                            videos
                                .results
                                .iter()
                                .filter(|video| {
                                    video.site.eq_ignore_ascii_case("youtube")
                                        && matches!(video.video_type.as_str(), "Trailer" | "Teaser")
                                })
                                .take(6)
                                .cloned()
                                .collect()
                        })
                        .unwrap_or_default();
                    LoadedTitleDetails {
                        imdb_id: movie.imdb_id.clone(),
                        title: movie.title,
                        overview: movie.overview,
                        tagline: movie.tagline,
                        release_date: movie.release_date,
                        runtime: movie.runtime,
                        status: movie.status,
                        rating: movie.vote_average,
                        rating_count: movie.vote_count,
                        genres: movie.genres.into_iter().map(|genre| genre.name).collect(),
                        cast,
                        crew,
                        production_companies: movie
                            .production_companies
                            .into_iter()
                            .map(|company| company.name)
                            .take(8)
                            .collect(),
                        production_countries: movie
                            .production_countries
                            .into_iter()
                            .map(|country| country.name)
                            .take(8)
                            .collect(),
                        trailers,
                        related,
                        seasons: Vec::new(),
                        related_artwork_paths,
                        poster_path: movie.poster_path,
                        backdrop_path: movie.backdrop_path,
                        poster: None,
                        backdrop: None,
                    }
                }
                MediaType::Tv => {
                    let tv = metadata
                        .tv(id, &token)
                        .await
                        .map_err(|error| metadata_error_message(&error))?;
                    let credits = tv.credits.as_ref().or(tv.aggregate_credits.as_ref());
                    let cast = credits
                        .map(|credits| {
                            credits
                                .cast
                                .iter()
                                .take(18)
                                .map(|person| {
                                    (
                                        person.id,
                                        person.name.clone(),
                                        person.character.clone().unwrap_or_else(|| "Cast".into()),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let crew = credits
                        .map(|credits| {
                            credits
                                .crew
                                .iter()
                                .filter(|person| {
                                    matches!(
                                        person.department.as_deref(),
                                        Some("Directing" | "Writing" | "Production")
                                    )
                                })
                                .take(12)
                                .map(|person| {
                                    (
                                        person.id,
                                        person.name.clone(),
                                        person
                                            .job
                                            .clone()
                                            .or(person.department.clone())
                                            .unwrap_or_else(|| "Crew".into()),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let related_source = tv
                        .recommendations
                        .as_ref()
                        .filter(|list| !list.results.is_empty())
                        .or(tv.similar.as_ref());
                    let mut related = Vec::new();
                    let mut related_artwork_paths = Vec::new();
                    if let Some(list) = related_source {
                        for item in list.results.iter().take(12) {
                            let kind = item.media_type.unwrap_or(MediaType::Tv);
                            if !matches!(kind, MediaType::Movie | MediaType::Tv) {
                                continue;
                            }
                            let detail = item
                                .release_year()
                                .map(|year| format!("TV SERIES · {year}"))
                                .unwrap_or_else(|| "TV SERIES".to_owned());
                            if let Some(path) = item.poster_path.clone() {
                                related_artwork_paths.push((related.len(), path));
                            }
                            related.push(SearchMediaCard {
                                id: item.id,
                                media_type: kind,
                                title: item.display_title().to_owned(),
                                detail,
                                overview: item.overview.clone(),
                                vote_average: item.vote_average,
                                poster: None,
                            });
                        }
                    }
                    let trailers = tv
                        .videos
                        .as_ref()
                        .map(|videos| {
                            videos
                                .results
                                .iter()
                                .filter(|video| {
                                    video.site.eq_ignore_ascii_case("youtube")
                                        && matches!(video.video_type.as_str(), "Trailer" | "Teaser")
                                })
                                .take(6)
                                .cloned()
                                .collect()
                        })
                        .unwrap_or_default();
                    LoadedTitleDetails {
                        imdb_id: tv.external_ids.as_ref().and_then(|ids| ids.imdb_id.clone()),
                        title: tv.name,
                        overview: tv.overview,
                        tagline: tv.tagline,
                        release_date: tv.first_air_date,
                        runtime: tv
                            .episode_run_time
                            .as_ref()
                            .and_then(|values| values.first())
                            .copied(),
                        status: tv.status,
                        rating: tv.vote_average,
                        rating_count: tv.vote_count,
                        genres: tv.genres.into_iter().map(|genre| genre.name).collect(),
                        cast,
                        crew,
                        production_companies: tv
                            .production_companies
                            .into_iter()
                            .map(|company| company.name)
                            .take(8)
                            .collect(),
                        production_countries: tv
                            .production_countries
                            .into_iter()
                            .map(|country| country.name)
                            .take(8)
                            .collect(),
                        trailers,
                        related,
                        seasons: tv.seasons,
                        related_artwork_paths,
                        poster_path: tv.poster_path,
                        backdrop_path: tv.backdrop_path,
                        poster: None,
                        backdrop: None,
                    }
                }
                _ => return Err("Unsupported media type".to_owned()),
            };

            if let Some(path) = loaded.poster_path.take() {
                loaded.poster = load_gpui_artwork(
                    artwork.clone(),
                    path,
                    ArtworkSize::W500,
                    token.child_token(),
                )
                .await;
            }
            if let Some(path) = loaded.backdrop_path.take() {
                loaded.backdrop = load_gpui_artwork(
                    artwork.clone(),
                    path,
                    ArtworkSize::W1280,
                    token.child_token(),
                )
                .await;
            }

            let mut jobs = JoinSet::new();
            for (index, path) in loaded.related_artwork_paths.drain(..) {
                let image_service = artwork.clone();
                let image_token = token.child_token();
                jobs.spawn(async move {
                    let image =
                        load_gpui_artwork(image_service, path, ArtworkSize::W342, image_token)
                            .await;
                    (index, image)
                });
            }
            while let Some(result) = jobs.join_next().await {
                if token.is_cancelled() {
                    return Err("Cancelled".to_owned());
                }
                if let Ok((index, Some(image))) = result
                    && let Some(card) = loaded.related.get_mut(index)
                {
                    card.poster = Some(image);
                }
            }

            if token.is_cancelled() {
                return Err("Cancelled".to_owned());
            }
            Ok::<_, String>(loaded)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await.unwrap_or_else(|_| {
                Err(
                    "The details request stopped unexpectedly. Check your connection and retry."
                        .to_owned(),
                )
            });
            let _ = this.update(cx, |this, cx| {
                if cancellation.is_cancelled() {
                    return;
                }
                let Some(details) = this
                    .details
                    .as_mut()
                    .filter(|details| details.id == id && details.media_type == media_type)
                else {
                    return;
                };
                let mut should_load_sources = false;
                match result {
                    Ok(loaded) => {
                        should_load_sources = true;
                        details.imdb_id = loaded.imdb_id;
                        details.title = loaded.title;
                        details.overview = loaded.overview;
                        details.tagline = loaded.tagline;
                        details.release_date = loaded.release_date;
                        details.runtime = loaded.runtime;
                        details.status = loaded.status;
                        details.rating = loaded.rating;
                        details.rating_count = loaded.rating_count;
                        details.genres = loaded.genres;
                        details.cast = loaded.cast;
                        details.crew = loaded.crew;
                        details.production_companies = loaded.production_companies;
                        details.production_countries = loaded.production_countries;
                        details.trailers = loaded.trailers;
                        details.related = loaded.related;
                        details.seasons = loaded.seasons;
                        details.poster = loaded.poster;
                        details.backdrop = loaded.backdrop;
                        details.loading = false;
                        details.error = None;
                    }
                    Err(error) => {
                        details.loading = false;
                        details.error = Some(error);
                    }
                }
                if should_load_sources {
                    this.load_detail_sources(None, None, false, cx);
                    if let Some((pending_id, pending_season, _pending_episode)) =
                        this.pending_continue_watching_episode
                        && pending_id == id
                    {
                        this.load_detail_season(pending_season, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn load_detail_sources(
        &mut self,
        season: Option<u32>,
        episode: Option<u32>,
        force_refresh: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(details) = self.details.as_mut() else {
            return;
        };
        if details.loading || details.media_type == MediaType::Person {
            return;
        }
        let id = details.id;
        let media_type = if details.media_type == MediaType::Movie {
            "movie"
        } else {
            "tv"
        };
        let title = details.title.clone();
        let year = details
            .release_date
            .as_deref()
            .and_then(|date| date.get(..4))
            .and_then(|year| year.parse::<u16>().ok());
        let imdb_id = details.imdb_id.clone();
        let mut media_id = imdb_id.clone().unwrap_or_else(|| format!("tmdb:{id}"));
        if let (Some(season), Some(episode)) = (season, episode) {
            media_id.push_str(&format!(":{season}:{episode}"));
        }
        let request = SourceRequest {
            media_type: media_type.to_owned(),
            media_id,
            tmdb_id: Some(id.to_string()),
            imdb_id,
            title,
            year,
            season,
            episode,
        };
        let request_key = format!("{id}:{media_type}:{season:?}:{episode:?}");
        details.source_loading = true;
        details.source_error = None;
        details.source_result = None;
        details.source_season = season;
        details.source_episode = episode;
        details.resolved_debrid_source = None;
        details.resolver_source_title = None;
        details.resolver_error = None;
        details.resolver_loading = false;
        let aggregator = self.source_aggregator.clone();
        let cancellation = self.details_cancellation.child_token();
        let completion_cancellation = cancellation.clone();
        let task = self
            .tokio
            .spawn(async move { aggregator.discover(request, force_refresh).await });
        cx.spawn(async move |this, cx| {
            let result = task
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result.map_err(|error| error.to_string()));
            let _ = this.update(cx, |this, cx| {
                if completion_cancellation.is_cancelled() {
                    return;
                }
                let Some(details) = this.details.as_mut().filter(|details| {
                    format!(
                        "{}:{}:{:?}:{:?}",
                        details.id,
                        if details.media_type == MediaType::Movie {
                            "movie"
                        } else {
                            "tv"
                        },
                        details.source_season,
                        details.source_episode
                    ) == request_key
                }) else {
                    return;
                };
                details.source_loading = false;
                match result {
                    Ok(result) => {
                        details.source_result = Some(result);
                        details.source_error = None;
                    }
                    Err(error) => {
                        details.source_error = Some(error);
                        details.source_result = None;
                    }
                }
                let should_auto_play = this.autoplay_next_pending
                    && this.autoplay_next_target == Some((details.id, season.unwrap_or(0), episode.unwrap_or(0)));
                let discovered_sources = details.source_result.clone();
                if should_auto_play {
                    let preferred_provider = this.playback_preferred_provider_id.clone();
                    let candidate = discovered_sources.as_ref().and_then(|result| {
                        let mut sources = result.groups.iter()
                            .flat_map(|group| group.sources.iter())
                            .filter(|source| matches!(source.target, SourceTarget::Http(_)))
                            .collect::<Vec<_>>();
                        sources.sort_by_key(|source| {
                            if preferred_provider.as_deref() == Some(source.provider_id.as_str()) { 0 } else { 1 }
                        });
                        sources.first().map(|source| match &source.target {
                            SourceTarget::Http(url) => (url.clone(), source.headers.clone()),
                            _ => unreachable!(),
                        })
                    });
                    this.autoplay_next_pending = false;
                    this.autoplay_next_target = None;
                    if let Some((url, headers)) = candidate {
                        this.play_source(url, headers, cx);
                    } else {
                        this.notification = Some("The next episode has no direct source ready. Choose a source manually.".to_owned());
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn play_source(
        &mut self,
        url: String,
        headers: std::collections::BTreeMap<String, String>,
        cx: &mut Context<Self>,
    ) {
        let mut sources = self
            .details
            .as_ref()
            .and_then(|details| details.source_result.as_ref())
            .map(|result| {
                result.groups.iter().flat_map(|group| group.sources.iter())
                    .filter_map(|source| match &source.target {
                        SourceTarget::Http(source_url) => Some(PlaybackSourceCandidate {
                            url: source_url.clone(),
                            headers: source.headers.clone(),
                            title: source.title.clone(),
                            provider_id: source.provider_id.clone(),
                            provider_name: source.provider_name.clone(),
                        }),
                        _ => None,
                    }).collect::<Vec<_>>()
            }).unwrap_or_default();
        if !sources.iter().any(|source| source.url == url) {
            sources.insert(0, PlaybackSourceCandidate {
                url: url.clone(),
                headers: headers.clone(),
                title: "Resolved source".to_owned(),
                provider_id: "resolved".to_owned(),
                provider_name: "Resolved source".to_owned(),
            });
        }
        self.playback_source_index = sources.iter().position(|source| source.url == url).unwrap_or(0);
        self.playback_preferred_provider_id = sources.get(self.playback_source_index)
            .map(|source| source.provider_id.clone());
        self.playback_sources = sources;
        self.playback_open = true;
        self.playback_panel = None;
        self.playback_error = None;
        self.playback_end_handled = false;
        self.playback_resume_position_seconds = None;
        self.playback_context = self.details.as_ref().and_then(|details| match details.media_type {
            MediaType::Movie => Some(PlaybackContext {
                media_id: details.id.to_string(),
                media_kind: MediaKind::Movie,
                title: details.title.clone(),
                season_number: None,
                episode_number: None,
                episode_title: None,
                still_path: None,
                last_persisted_position_ms: 0,
            }),
            MediaType::Tv => details.selected_episode.as_ref().map(|episode| PlaybackContext {
                media_id: details.id.to_string(),
                media_kind: MediaKind::Series,
                title: details.title.clone(),
                season_number: Some(episode.season_number),
                episode_number: Some(episode.episode_number),
                episode_title: Some(episode.name.clone()),
                still_path: episode.still_path.clone(),
                last_persisted_position_ms: 0,
            }),
            MediaType::Person | MediaType::Other => None,
        });
        self.playback_resume_prompt = None;
        self.playback_resume_source = None;
        let selected_source = self.playback_sources[self.playback_source_index].clone();
        if let Some(context) = self.playback_context.as_ref() {
            let saved = self.playback_progress_repository.get_playback_progress(
                "default", &context.media_id, context.media_kind,
                context.season_number, context.episode_number,
            ).ok().flatten();
            if let Some(progress) = saved.filter(|progress| !progress.is_completed && progress.position_ms >= 30_000) {
                self.playback_resume_prompt = Some(progress);
                self.playback_resume_source = Some(selected_source);
                self.playback_loading = false;
                self.reveal_playback_controls(cx);
                cx.notify();
                return;
            }
        }
        self.reveal_playback_controls(cx);
        self.start_playback_url(url, headers, cx);
    }

    fn choose_resume_action(&mut self, resume: bool, cx: &mut Context<Self>) {
        let Some(source) = self.playback_resume_source.take() else {
            self.playback_resume_prompt = None;
            return;
        };
        self.playback_resume_position_seconds = if resume {
            self.playback_resume_prompt.as_ref()
                .map(|progress| progress.position_ms as f64 / 1000.0)
        } else { None };
        self.playback_resume_prompt = None;
        self.start_playback_url(source.url, source.headers, cx);
    }

    fn persist_playback_snapshot(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some(context) = self.playback_context.as_mut() else { return };
        let Some(snapshot) = self.mpv_player.as_ref().map(MpvPlayer::snapshot) else { return };
        let position_ms = (snapshot.position_seconds.unwrap_or_default().max(0.0) * 1000.0)
            .min(u64::MAX as f64) as u64;
        let duration_ms = (snapshot.duration_seconds.unwrap_or_default().max(0.0) * 1000.0)
            .min(u64::MAX as f64) as u64;
        let completed = playback_is_complete(position_ms, duration_ms, snapshot.status == PlaybackStatus::Ended);
        if !should_store_playback_progress(position_ms, duration_ms) && !completed { return; }
        if !force && !completed && position_ms.saturating_sub(context.last_persisted_position_ms) < 5_000 { return; }
        context.last_persisted_position_ms = position_ms;
        let progress = PlaybackProgress {
            profile_id: "default".to_owned(),
            media_id: context.media_id.clone(),
            media_kind: context.media_kind,
            title: context.title.clone(),
            season_number: context.season_number,
            episode_number: context.episode_number,
            episode_title: context.episode_title.clone(),
            poster_path: None,
            backdrop_path: None,
            still_path: context.still_path.clone(),
            position_ms,
            duration_ms,
            is_completed: completed,
            updated_at_ms: current_epoch_millis(),
        };
        let refresh_up_next = completed;
        let repository = self.playback_progress_repository.clone();
        let runtime = self.tokio.clone();
        let task = runtime.spawn_blocking(move || {
            repository.save_playback_progress(&progress)?;
            repository.list_continue_watching("default", 20)
        });
        cx.spawn(async move |this, cx| {
            match task.await {
                Ok(Ok(entries)) => {
                    let _ = this.update(cx, |this, cx| {
                        this.home_continue_watching = entries;
                        this.home_progress_artwork_requested = false;
                        this.load_home_progress_artwork(cx);
                        if refresh_up_next {
                            this.home_up_next_requested = false;
                            this.load_home_up_next(cx);
                        }
                        cx.notify();
                    });
                }
                Ok(Err(error)) => tracing::warn!(error = %error, "could not persist playback progress"),
                Err(error) => tracing::warn!(error = %error, "playback progress task failed"),
            }
        }).detach();
    }

    fn toggle_autoplay_next(&mut self, cx: &mut Context<Self>) {
        self.playback_autoplay_next = !self.playback_autoplay_next;
        let key = SettingKey::new("playback.autoplay_next").expect("valid autoplay setting key");
        if let Err(error) = self.settings.set(&key, if self.playback_autoplay_next { "true" } else { "false" }) {
            tracing::warn!(error = %error, "could not save autoplay-next preference");
        }
        self.notification = Some(if self.playback_autoplay_next {
            "Autoplay next episode enabled.".to_owned()
        } else { "Autoplay next episode disabled.".to_owned() });
        cx.notify();
    }

    fn open_continue_watching(&mut self, progress: PlaybackProgress, cx: &mut Context<Self>) {
        let Ok(id) = progress.media_id.parse::<u64>() else {
            self.notification = Some("This saved title has an unsupported metadata ID.".to_owned());
            cx.notify();
            return;
        };
        if progress.media_kind == MediaKind::Series {
            if let (Some(season), Some(episode)) = (progress.season_number, progress.episode_number) {
                self.pending_continue_watching_episode = Some((id, season, episode));
            }
            self.load_details(id, MediaType::Tv, progress.title, cx);
        } else {
            self.pending_continue_watching_episode = None;
            self.load_details(id, MediaType::Movie, progress.title, cx);
        }
    }

    fn toggle_selected_episode_watched(&mut self, cx: &mut Context<Self>) {
        let Some(details) = self.details.as_ref() else { return };
        let Some(episode) = details.selected_episode.as_ref() else { return };
        let item = WatchedItem {
            profile_id: "default".to_owned(),
            media_id: details.id.to_string(),
            media_kind: MediaKind::Series,
            season_number: Some(episode.season_number),
            episode_number: Some(episode.episode_number),
            marked_at_ms: current_epoch_millis(),
        };
        let watched = self.playback_progress_repository.is_watched(&item).unwrap_or(false);
        match self.playback_progress_repository.set_watched(&item, !watched) {
            Ok(()) => {
                self.notification = Some(if watched {
                    "Episode removed from watched history.".to_owned()
                } else { "Episode marked as watched.".to_owned() });
                self.home_continue_watching = self.playback_progress_repository
                    .list_continue_watching("default", 20).unwrap_or_default();
                self.home_progress_artwork_requested = false;
                self.load_home_progress_artwork(cx);
                self.home_up_next_requested = false;
                self.load_home_up_next(cx);
            }
            Err(error) => self.notification = Some(format!("Could not update watched state: {error}")),
        }
        cx.notify();
    }

    fn navigate_selected_episode(&mut self, direction: isize, autoplay: bool, cx: &mut Context<Self>) {
        let Some(details) = self.details.as_ref() else { return };
        if details.media_type != MediaType::Tv { return; }
        let Some(current_episode) = details.selected_episode.as_ref() else { return };
        let current_season = current_episode.season_number;
        let current_number = current_episode.episode_number;
        let show_id = details.id;
        let target_release_date = details.selected_season.as_ref().and_then(|season| {
            season.episodes.iter()
                .find(|episode| direction > 0 && episode.episode_number == current_number.saturating_add(1)
                    || direction < 0 && episode.episode_number.saturating_add(1) == current_number)
                .and_then(|episode| episode.air_date.clone())
        });
        let season_summaries = details.seasons.clone();
        let same_season_target = details.selected_season.as_ref().and_then(|season| {
            let mut numbers = season.episodes.iter().map(|episode| episode.episode_number).collect::<Vec<_>>();
            numbers.sort_unstable();
            let position = numbers.iter().position(|number| *number == current_number)?;
            if direction > 0 { numbers.get(position + 1).copied() }
            else { position.checked_sub(1).and_then(|index| numbers.get(index).copied()) }
        });
        if let Some(target_episode) = same_season_target {
            if let Some(date) = target_release_date.as_deref().filter(|date| date.len() >= 10)
                && date > current_utc_date().as_str()
            {
                self.notification = Some(format!("The next episode releases on {date}."));
                cx.notify();
                return;
            }
            self.autoplay_next_pending = autoplay;
            self.autoplay_next_target = autoplay.then_some((show_id, current_season, target_episode));
            self.load_detail_episode(current_season, target_episode, cx);
            return;
        }
        let mut seasons = season_summaries.iter().map(|season| season.season_number)
            .filter(|number| *number > 0).collect::<Vec<_>>();
        seasons.sort_unstable();
        seasons.dedup();
        let target_season = if direction > 0 {
            seasons.into_iter().find(|number| *number > current_season)
        } else {
            seasons.into_iter().filter(|number| *number < current_season).next_back()
        };
        let Some(target_season) = target_season else {
            self.notification = Some("There are no more episodes in this direction.".to_owned());
            cx.notify();
            return;
        };
        self.autoplay_next_pending = autoplay;
        let cancellation = self.details_cancellation.child_token();
        let artwork = self.artwork.clone();
        let metadata = self.metadata.clone();
        let task_token = cancellation.clone();
        let task = self.tokio.spawn(async move {
            let season = metadata.season(show_id, target_season, &task_token).await
                .map_err(|error| metadata_error_message(&error))?;
            let poster = match season.poster_path.as_deref() {
                Some(path) => load_gpui_artwork(artwork, path.to_owned(), ArtworkSize::W500, task_token.child_token()).await,
                None => None,
            };
            Ok::<_, String>((season, poster))
        });
        cx.spawn(async move |this, cx| {
            match task.await {
                Ok(Ok((season, poster))) => {
                    let _ = this.update(cx, |this, cx| {
                        if cancellation.is_cancelled() { return; }
                        let target_episode = if direction > 0 {
                            season.episodes.iter().map(|episode| episode.episode_number).min()
                        } else {
                            season.episodes.iter().map(|episode| episode.episode_number).max()
                        };
                        let Some(target_episode) = target_episode else {
                            this.autoplay_next_pending = false;
                            this.notification = Some("The adjacent season has no episode entries.".to_owned());
                            cx.notify();
                            return;
                        };
                        let release_date = season.episodes.iter()
                            .find(|episode| episode.episode_number == target_episode)
                            .and_then(|episode| episode.air_date.as_deref());
                        if let Some(date) = release_date.filter(|date| date.len() >= 10)
                            && date > current_utc_date().as_str()
                        {
                            if let Some(details) = this.details.as_mut().filter(|details| details.id == show_id) {
                                details.selected_season_number = Some(target_season);
                                details.selected_season = Some(season);
                                details.selected_season_poster = poster;
                                details.selected_episode = None;
                                details.selected_episode_identity = None;
                                details.selected_episode_artwork = None;
                                details.season_loading = false;
                                details.season_error = None;
                            }
                            this.autoplay_next_pending = false;
                            this.notification = Some(format!("The next episode releases on {date}."));
                            cx.notify();
                            return;
                        }
                        if let Some(details) = this.details.as_mut().filter(|details| details.id == show_id) {
                            details.selected_season_number = Some(target_season);
                            details.selected_season = Some(season);
                            details.selected_season_poster = poster;
                            details.selected_episode = None;
                            details.selected_episode_identity = None;
                            details.selected_episode_artwork = None;
                            details.season_loading = false;
                            details.season_error = None;
                        }
                        this.autoplay_next_target = Some((show_id, target_season, target_episode));
                        this.load_detail_episode(target_season, target_episode, cx);
                    });
                }
                Ok(Err(error)) => {
                    let _ = this.update(cx, |this, cx| {
                        this.autoplay_next_pending = false;
                        this.notification = Some(format!("Could not load the adjacent season: {error}"));
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.autoplay_next_pending = false;
                        this.notification = Some(format!("Episode navigation failed: {error}"));
                        cx.notify();
                    });
                }
            }
        }).detach();
        cx.notify();
    }

    fn start_playback_url(
        &mut self,
        url: String,
        headers: BTreeMap<String, String>,
        cx: &mut Context<Self>,
    ) {
        if self.playback_loading {
            return;
        }
        self.playback_open = true;
        self.playback_error = None;
        self.playback_loading = true;
        self.playback_end_handled = false;
        let resume_position = self.playback_resume_position_seconds.take();
        self.playback_start_cancel.cancel();
        let start_cancel = CancellationToken::new();
        self.playback_start_cancel = start_cancel.clone();
        self.playback_refresh_cancel.cancel();

        let existing = self.mpv_player.clone();
        let runtime = self.tokio.clone();
        cx.notify();
        let task = runtime.spawn_blocking(move || {
            let player = match existing {
                Some(player) => player,
                None => MpvPlayer::spawn(MpvConfig::default())?,
            };
            let loaded = player.load(&url, &headers);
            if loaded.is_ok()
                && let Some(position) = resume_position.filter(|position| position.is_finite() && *position >= 0.0)
            {
                let deadline = Instant::now() + Duration::from_secs(8);
                while Instant::now() < deadline {
                    let snapshot = player.snapshot();
                    if snapshot.duration_seconds.is_some_and(|duration| duration > 0.0)
                        && matches!(snapshot.status, PlaybackStatus::Playing | PlaybackStatus::Paused)
                    {
                        let _ = player.seek_absolute(position);
                        break;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
            Ok::<_, senzo_player::MpvError>((player, loaded))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if start_cancel.is_cancelled() {
                    return;
                }
                this.playback_loading = false;
                match result {
                    Ok(Ok((player, Ok(())))) => {
                        this.mpv_player = Some(player);
                        this.playback_error = None;
                        this.start_playback_refresh(cx);
                    }
                    Ok(Ok((player, Err(error)))) => {
                        this.mpv_player = Some(player);
                        if this.playback_source_index + 1 < this.playback_sources.len() {
                            this.playback_error =
                                Some(format!("{error}; trying the next direct source"));
                            this.try_next_playback_source(cx);
                        } else {
                            this.playback_error = Some(error.to_string());
                        }
                    }
                    Ok(Err(error)) => this.playback_error = Some(error.to_string()),
                    Err(error) => {
                        this.playback_error = Some(format!("MPV worker task failed: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn start_playback_refresh(&mut self, cx: &mut Context<Self>) {
        self.playback_refresh_cancel.cancel();
        let cancel = CancellationToken::new();
        self.playback_refresh_cancel = cancel.clone();
        let background_cancel = cancel.clone();
        let (refresh_tx, mut refresh_rx) = mpsc::unbounded_channel::<()>();
        drop(self.tokio.spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_millis(250));
            loop {
                tokio::select! {
                    _ = background_cancel.cancelled() => break,
                    _ = ticker.tick() => {
                        if refresh_tx.send(()).is_err() { break; }
                    }
                }
            }
        }));
        cx.spawn(async move |this, cx| {
            while refresh_rx.recv().await.is_some() {
                let keep_refreshing = this.update(cx, |this, cx| {
                    let snapshot = this.mpv_player.as_ref().map(MpvPlayer::snapshot);
                    if let Some(snapshot) = snapshot {
                        if snapshot.status == PlaybackStatus::Ended && !this.playback_end_handled {
                            this.playback_end_handled = true;
                            this.persist_playback_snapshot(true, cx);
                            if this.playback_autoplay_next {
                                this.navigate_selected_episode(1, true, cx);
                            }
                        } else {
                            this.persist_playback_snapshot(false, cx);
                        }
                    }
                    let active = this.mpv_player.as_ref().is_some_and(|player| {
                        matches!(
                            player.snapshot().status,
                            PlaybackStatus::Loading
                                | PlaybackStatus::Playing
                                | PlaybackStatus::Paused
                        )
                    });
                    cx.notify();
                    active
                });
                if !matches!(keep_refreshing, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    fn run_player_command(
        &mut self,
        command: impl FnOnce(MpvPlayer) -> Result<(), senzo_player::MpvError> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(player) = self.mpv_player.clone() else {
            return;
        };
        let runtime = self.tokio.clone();
        let task = runtime.spawn_blocking(move || command(player));
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(Ok(())) => this.playback_error = None,
                    Ok(Err(error)) => this.playback_error = Some(error.to_string()),
                    Err(error) => {
                        this.playback_error = Some(format!("MPV command task failed: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_pause_playback(&mut self, cx: &mut Context<Self>) {
        let Some(paused) = self
            .mpv_player
            .as_ref()
            .map(|player| player.snapshot().paused)
        else {
            return;
        };
        self.run_player_command(
            move |player| {
                if paused {
                    player.play()
                } else {
                    player.pause()
                }
            },
            cx,
        );
    }

    fn seek_playback(&mut self, seconds: f64, cx: &mut Context<Self>) {
        self.run_player_command(move |player| player.seek_relative(seconds), cx);
    }

    fn adjust_playback_volume(&mut self, delta: f64, cx: &mut Context<Self>) {
        let Some(volume) = self
            .mpv_player
            .as_ref()
            .map(|player| player.snapshot().volume_percent)
        else {
            return;
        };
        let target = (volume + delta).clamp(0.0, 100.0);
        self.run_player_command(move |player| player.set_volume(target), cx);
    }

    fn toggle_playback_mute(&mut self, cx: &mut Context<Self>) {
        let Some(muted) = self
            .mpv_player
            .as_ref()
            .map(|player| player.snapshot().muted)
        else {
            return;
        };
        self.run_player_command(move |player| player.set_muted(!muted), cx);
    }

    fn toggle_playback_speed(&mut self, cx: &mut Context<Self>) {
        let Some(speed) = self
            .mpv_player
            .as_ref()
            .map(|player| player.snapshot().speed)
        else {
            return;
        };
        let target = if (speed - 1.0).abs() < 0.01 {
            1.25
        } else {
            1.0
        };
        self.run_player_command(move |player| player.set_speed(target), cx);
    }

    fn select_playback_track(&mut self, kind: TrackKind, id: i64, cx: &mut Context<Self>) {
        self.run_player_command(
            move |player| match kind {
                TrackKind::Audio => player.select_audio_track(Some(id)),
                TrackKind::Subtitle => player.select_subtitle_track(Some(id)),
            },
            cx,
        );
    }

    fn stop_playback(&mut self) {
        self.playback_start_cancel.cancel();
        self.playback_loading = false;
        self.playback_refresh_cancel.cancel();
        self.playback_controls_timer.cancel();
        self.playback_open = false;
        self.playback_controls_visible = false;
        self.playback_panel = None;
        let restore_pip = self.playback_picture_in_picture;
        let restore_fullscreen = self.playback_fullscreen;
        self.playback_picture_in_picture = false;
        self.playback_fullscreen = false;
        if let Some(player) = self.mpv_player.clone() {
            let runtime = self.tokio.clone();
            drop(runtime.spawn_blocking(move || {
                if restore_pip {
                    let _ = player.set_picture_in_picture(false);
                }
                if restore_fullscreen {
                    let _ = player.set_fullscreen(false);
                }
                let _ = player.stop(Duration::from_secs(6));
            }));
        }
    }

    fn close_player(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.persist_playback_snapshot(true, cx);
        self.stop_playback();
        self.playback_scrubbing = false;
        self.playback_fullscreen = false;
        self.playback_picture_in_picture = false;
        window.focus(
            self.navigation_focus
                .get(&self.navigation.current())
                .expect("current page focus handle exists"),
            cx,
        );
        cx.notify();
    }

    fn reveal_playback_controls(&mut self, cx: &mut Context<Self>) {
        self.playback_controls_visible = true;
        self.playback_controls_timer.cancel();
        let cancel = CancellationToken::new();
        self.playback_controls_timer = cancel.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            if cancel.is_cancelled() {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if this.playback_open
                    && !matches!(
                        this.playback_panel,
                        Some(PlayerPanel::Audio | PlayerPanel::Subtitles)
                    )
                {
                    this.playback_controls_visible = false;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn seek_playback_absolute(&mut self, seconds: f64, cx: &mut Context<Self>) {
        self.run_player_command(move |player| player.seek_absolute(seconds), cx);
    }

    fn retry_playback(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self
            .playback_sources
            .get(self.playback_source_index)
            .cloned()
        else {
            return;
        };
        self.start_playback_url(source.url, source.headers, cx);
    }

    fn try_next_playback_source(&mut self, cx: &mut Context<Self>) {
        let next = self.playback_source_index.saturating_add(1);
        let Some(source) = self.playback_sources.get(next).cloned() else {
            self.playback_error = Some("No additional direct sources are available. Refresh source discovery and try again.".to_owned());
            cx.notify();
            return;
        };
        self.playback_source_index = next;
        self.start_playback_url(source.url, source.headers, cx);
    }

    fn toggle_player_fullscreen(&mut self, cx: &mut Context<Self>) {
        let next = !self.playback_fullscreen;
        self.playback_fullscreen = next;
        self.run_player_command(move |player| player.set_fullscreen(next), cx);
    }

    fn toggle_player_picture_in_picture(&mut self, cx: &mut Context<Self>) {
        let next = !self.playback_picture_in_picture;
        self.playback_picture_in_picture = next;
        self.run_player_command(move |player| player.set_picture_in_picture(next), cx);
    }

    fn cycle_player_resize_mode(&mut self, cx: &mut Context<Self>) {
        self.playback_resize_mode = match self.playback_resize_mode {
            ResizeMode::Fit => ResizeMode::Fill,
            ResizeMode::Fill => ResizeMode::Stretch,
            ResizeMode::Stretch => ResizeMode::Zoom,
            ResizeMode::Zoom => ResizeMode::Fit,
        };
        let mode = self.playback_resize_mode;
        self.run_player_command(move |player| player.set_resize_mode(mode), cx);
    }

    fn adjust_subtitle_font_size(&mut self, delta: i32, cx: &mut Context<Self>) {
        self.playback_subtitle_font_size =
            (self.playback_subtitle_font_size as i32 + delta).clamp(12, 72) as u32;
        let size = self.playback_subtitle_font_size;
        self.run_player_command(move |player| player.set_subtitle_font_size(size), cx);
    }

    fn adjust_subtitle_delay(&mut self, delta: f64, cx: &mut Context<Self>) {
        self.playback_subtitle_delay_seconds =
            (self.playback_subtitle_delay_seconds + delta).clamp(-60.0, 60.0);
        let delay = self.playback_subtitle_delay_seconds;
        self.run_player_command(move |player| player.set_subtitle_delay(delay), cx);
    }

    fn toggle_subtitle_visibility(&mut self, cx: &mut Context<Self>) {
        self.playback_subtitles_visible = !self.playback_subtitles_visible;
        let visible = self.playback_subtitles_visible;
        self.run_player_command(move |player| player.set_subtitles_visible(visible), cx);
    }

    fn add_external_subtitle(&mut self, cx: &mut Context<Self>) {
        let url = self.subtitle_url_input.read(cx).value().trim().to_owned();
        if url.is_empty() {
            self.playback_error = Some("Enter a direct HTTP(S) subtitle URL first.".to_owned());
            cx.notify();
            return;
        }
        self.playback_error = None;
        self.run_player_command(
            move |player| player.add_external_subtitle(&url, &BTreeMap::new()),
            cx,
        );
    }

    fn resolve_detail_torrent(
        &mut self,
        source: Source,
        provider: DebridProviderId,
        cx: &mut Context<Self>,
    ) {
        let Some(details) = self.details.as_mut() else {
            return;
        };
        let season = details.source_season;
        let episode = details.source_episode;
        let id = details.id;
        details.resolver_loading = true;
        details.resolver_error = None;
        details.resolved_debrid_source = None;
        details.resolver_source_title = Some(source.title.clone());
        let aggregator = self.source_aggregator.clone();
        let cancellation = self.details_cancellation.child_token();
        let completion_cancellation = cancellation.clone();
        let task = self.tokio.spawn(async move {
            aggregator
                .resolve_torrent("default", &source, provider, season, episode)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = task
                .await
                .map_err(|error| error.to_string())
                .and_then(|value| value.map_err(|error| error.to_string()));
            let _ = this.update(cx, |this, cx| {
                if completion_cancellation.is_cancelled() {
                    return;
                }
                if let Some(details) = this.details.as_mut().filter(|details| details.id == id) {
                    details.resolver_loading = false;
                    match result {
                        Ok(resolved) => {
                            details.resolved_debrid_source = Some(resolved);
                            details.resolver_error = None;
                        }
                        Err(error) => details.resolver_error = Some(error),
                    }
                    cx.notify();
                }
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

    fn close_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.stop_playback();
        self.details_cancellation.cancel();
        self.details = None;
        if let Some(previous) = self.detail_history.pop() {
            match previous {
                DetailRoute::Title {
                    id,
                    media_type,
                    title,
                } => self.load_details(id, media_type, title, cx),
                DetailRoute::Person { id, name, profile } => {
                    self.load_person_details(id, name, profile, cx)
                }
            }
            return;
        }
        let page = self.navigation.current();
        let focus_id = self.home_return_focus_id.take();
        self.restore_page_position(page, focus_id, window);
        cx.notify();
    }

    fn push_current_detail_route(&mut self) {
        if let Some(details) = self.details.as_ref() {
            self.detail_history.push(DetailRoute::Title {
                id: details.id,
                media_type: details.media_type,
                title: details.title.clone(),
            });
        } else if let Some(person) = self.person_details.as_ref() {
            self.detail_history.push(DetailRoute::Person {
                id: person.id,
                name: person.name.clone(),
                profile: person.profile.clone(),
            });
        }
        const MAX_DETAIL_HISTORY: usize = 32;
        if self.detail_history.len() > MAX_DETAIL_HISTORY {
            self.detail_history.remove(0);
        }
    }

    fn open_related_detail(
        &mut self,
        id: u64,
        media_type: MediaType,
        title: String,
        cx: &mut Context<Self>,
    ) {
        self.stop_playback();
        if !matches!(media_type, MediaType::Movie | MediaType::Tv) {
            return;
        }
        self.push_current_detail_route();
        self.load_details(id, media_type, title, cx);
    }

    fn open_detail_person(&mut self, id: u64, name: String, cx: &mut Context<Self>) {
        if id == 0 {
            return;
        }
        self.push_current_detail_route();
        self.load_person_details(id, name, None, cx);
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
        let artwork = self.artwork.clone();
        if let Some(details) = self.details.as_mut() {
            details.season_cancellation.cancel();
            details.episode_cancellation.cancel();
            details.season_cancellation = cancellation.clone();
            details.selected_season_number = Some(number);
            details.selected_season = None;
            details.selected_season_poster = None;
            details.selected_episode = None;
            details.selected_episode_identity = None;
            details.selected_episode_artwork = None;
            details.episode_error = None;
            details.episode_loading = false;
            details.season_loading = true;
            details.season_error = None;
        }
        let task_token = cancellation.clone();
        let metadata = self.metadata.clone();
        let task = self.tokio.spawn(async move {
            let season = metadata
                .season(id, number, &task_token)
                .await
                .map_err(|error| metadata_error_message(&error))?;
            let poster = match season.poster_path.as_deref() {
                Some(path) => {
                    load_gpui_artwork(
                        artwork,
                        path.to_owned(),
                        ArtworkSize::W500,
                        task_token.child_token(),
                    )
                    .await
                }
                None => None,
            };
            Ok::<_, String>((season, poster))
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
                        Ok((season, poster)) => {
                            details.selected_season = Some(season);
                            details.selected_season_poster = poster;
                            details.season_error = None;
                        }
                        Err(error) => details.season_error = Some(error),
                    }
                    if let Some((pending_id, pending_season, pending_episode)) =
                        this.pending_continue_watching_episode
                        && pending_id == id
                        && pending_season == number
                        && this.details.as_ref().is_some_and(|details| details.selected_season.is_some())
                    {
                        this.pending_continue_watching_episode = None;
                        this.load_detail_episode(pending_season, pending_episode, cx);
                    }
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn load_detail_episode(
        &mut self,
        season_number: u32,
        episode_number: u32,
        cx: &mut Context<Self>,
    ) {
        let Some(current) = self.details.as_ref() else {
            return;
        };
        if current.media_type != MediaType::Tv
            || current.loading
            || current.selected_season_number != Some(season_number)
        {
            return;
        }
        let id = current.id;
        let cancellation = self.details_cancellation.child_token();
        if let Some(details) = self.details.as_mut() {
            details.episode_cancellation.cancel();
            details.episode_cancellation = cancellation.clone();
            details.selected_episode = None;
            details.selected_episode_identity = Some(crate::discovery::episode_identity(
                id,
                season_number,
                episode_number,
            ));
            details.selected_episode_artwork = None;
            details.episode_loading = true;
            details.episode_error = None;
        }
        let artwork = self.artwork.clone();
        let metadata = self.metadata.clone();
        let task_token = cancellation.clone();
        let task = self.tokio.spawn(async move {
            let episode = metadata
                .episode(id, season_number, episode_number, &task_token)
                .await
                .map_err(|error| metadata_error_message(&error))?;
            let still = match episode.still_path.as_deref() {
                Some(path) => {
                    load_gpui_artwork(
                        artwork,
                        path.to_owned(),
                        ArtworkSize::W780,
                        task_token.child_token(),
                    )
                    .await
                }
                None => None,
            };
            Ok::<_, String>((episode, still))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await.unwrap_or_else(|_| {
                Err(
                    "The episode request stopped unexpectedly. Check your connection and retry."
                        .to_owned(),
                )
            });
            let _ = this.update(cx, |this, cx| {
                if cancellation.is_cancelled() {
                    return;
                }
                if let Some(details) = this.details.as_mut().filter(|details| {
                    details.id == id && details.selected_season_number == Some(season_number)
                }) {
                    details.episode_loading = false;
                    match result {
                        Ok((episode, still)) => {
                            details.selected_episode = Some(episode);
                            details.selected_episode_artwork = still;
                            details.episode_error = None;
                        }
                        Err(error) => details.episode_error = Some(error),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
        self.load_detail_sources(Some(season_number), Some(episode_number), false, cx);
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
                .on_click(cx.listener(|this, _, window, cx| this.close_details(window, cx)))
                .child(div().text_sm().text_color(rgb(Theme::ACCENT)).child(
                    if self.detail_history.is_empty() {
                        "← Back to results"
                    } else {
                        "← Back"
                    },
                )),
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
            let votes = d
                .rating_count
                .map(|count| format!(" · {count} votes"))
                .unwrap_or_default();
            facts.push(format!("★ {rating:.1}/10{votes}"));
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
            for (id, name, role) in d.cast.iter().take(18) {
                let id = *id;
                let name_for_action = name.clone();
                row = row.child(
                    div()
                        .id(format!("title-cast-{id}"))
                        .tab_index(60)
                        .rounded_md()
                        .bg(rgb(Theme::SURFACE))
                        .p_3()
                        .min_w(px(140.0))
                        .cursor_pointer()
                        .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                        .aria_label(format!("View cast member {name}"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_detail_person(id, name_for_action.clone(), cx)
                        }))
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
        if !d.crew.is_empty() {
            let mut row = div().w_full().flex().flex_wrap().gap_3();
            for (id, name, role) in d.crew.iter().take(12) {
                let id = *id;
                let name_for_action = name.clone();
                row = row.child(
                    div()
                        .id(format!("title-crew-{id}"))
                        .tab_index(61)
                        .rounded_md()
                        .bg(rgb(Theme::SURFACE))
                        .p_3()
                        .min_w(px(140.0))
                        .cursor_pointer()
                        .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                        .aria_label(format!("View crew member {name}"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_detail_person(id, name_for_action.clone(), cx)
                        }))
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
                            .child("Crew"),
                    )
                    .child(row),
            );
        }
        if !d.production_companies.is_empty() || !d.production_countries.is_empty() {
            let mut production = div().w_full().flex().flex_col().gap_2().child(
                div()
                    .text_lg()
                    .text_color(rgb(Theme::TEXT_PRIMARY))
                    .child("Production"),
            );
            if !d.production_companies.is_empty() {
                production = production.child(
                    div()
                        .text_sm()
                        .text_color(rgb(Theme::TEXT_SECONDARY))
                        .child(format!(
                            "Companies · {}",
                            d.production_companies.join(" · ")
                        )),
                );
            }
            if !d.production_countries.is_empty() {
                production =
                    production.child(div().text_sm().text_color(rgb(Theme::TEXT_MUTED)).child(
                        format!("Countries · {}", d.production_countries.join(" · ")),
                    ));
            }
            page = page.child(production);
        }
        if !d.trailers.is_empty() {
            let mut trailers = div().w_full().flex().flex_col().gap_2().child(
                div()
                    .text_lg()
                    .text_color(rgb(Theme::TEXT_PRIMARY))
                    .child("Trailers & videos"),
            );
            for video in d.trailers.iter().take(6) {
                let kind = if video.official == Some(true) {
                    "Official"
                } else {
                    "Video"
                };
                trailers = trailers.child(
                    div()
                        .rounded_md()
                        .bg(rgb(Theme::SURFACE))
                        .p_3()
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(Theme::TEXT_PRIMARY))
                                .child(video.name.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(Theme::TEXT_MUTED))
                                .child(format!("{kind} · {} · {}", video.site, video.video_type)),
                        ),
                );
            }
            page = page.child(trailers);
        }
        if !d.related.is_empty() {
            let mut related = div().w_full().flex().flex_col().gap_3().child(
                div()
                    .text_lg()
                    .text_color(rgb(Theme::TEXT_PRIMARY))
                    .child("You may also like"),
            );
            let mut cards = div().w_full().flex().gap_4().overflow_x_scrollbar();
            for (index, card) in d.related.iter().enumerate() {
                let id = card.id;
                let kind = card.media_type;
                let title = card.title.clone();
                cards = cards.child(
                    div()
                        .id(format!("related-title-{id}-{index}"))
                        .tab_index(80 + index as isize)
                        .w(px(150.0))
                        .flex_shrink_0()
                        .cursor_pointer()
                        .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                        .aria_label(format!("Open related title {title}"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_related_detail(id, kind, title.clone(), cx)
                        }))
                        .child(poster_card(
                            &card.title,
                            &card.detail,
                            card.poster.clone(),
                            false,
                        )),
                );
            }
            related = related.child(cards);
            page = page.child(related);
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
                let season_name =
                    crate::discovery::season_label(season.season_number, &season.name);
                let mut selected_season =
                    div().w_full().flex().items_start().gap_4().child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_base()
                                    .text_color(rgb(Theme::TEXT_PRIMARY))
                                    .child(season_name),
                            )
                            .child(div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child(
                                format!(
                                    "{} episodes{}",
                                    season.episodes.len(),
                                    season
                                        .air_date
                                        .as_deref()
                                        .map(|date| format!(" · {date}"))
                                        .unwrap_or_default()
                                ),
                            )),
                    );
                if let Some(poster) = d.selected_season_poster.clone() {
                    selected_season = selected_season.child(
                        div()
                            .w(px(90.0))
                            .h(px(135.0))
                            .rounded_md()
                            .overflow_hidden()
                            .child(img(poster).w_full().h_full()),
                    );
                }
                section = section.child(selected_season);

                let mut episodes = div().w_full().flex().flex_col().gap_2();
                for episode in &season.episodes {
                    let episode_number = episode.episode_number;
                    let season_number = episode.season_number;
                    let selected = d.selected_episode_identity
                        == Some(crate::discovery::episode_identity(
                            d.id,
                            season_number,
                            episode_number,
                        ));
                    let mut heading = format!(
                        "S{:02} E{:02} · {}",
                        season_number, episode_number, episode.name
                    );
                    if let Some(minutes) = episode.runtime {
                        heading.push_str(&format!(" · {minutes} min"));
                    }
                    if let Some(date) = episode.air_date.as_deref() {
                        heading.push_str(&format!(" · {date}"));
                    }
                    episodes = episodes.child(
                        div()
                            .id(format!("episode-{season_number}-{episode_number}"))
                            .tab_index(70 + episode_number as isize)
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(if selected {
                                Theme::ACCENT
                            } else {
                                Theme::BORDER
                            }))
                            .bg(rgb(Theme::SURFACE))
                            .p_3()
                            .cursor_pointer()
                            .focus_visible(|style| {
                                style.border_1().border_color(rgb(Theme::ACCENT))
                            })
                            .aria_label(format!("View episode {heading}"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.load_detail_episode(season_number, episode_number, cx)
                            }))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(if selected {
                                        Theme::ACCENT
                                    } else {
                                        Theme::TEXT_PRIMARY
                                    }))
                                    .child(heading),
                            )
                            .child(
                                div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child(
                                    episode
                                        .overview
                                        .as_deref()
                                        .filter(|value| !value.trim().is_empty())
                                        .unwrap_or("No episode synopsis available.")
                                        .to_owned(),
                                ),
                            ),
                    );
                }
                section = section.child(episodes);
                if d.episode_loading {
                    section = section.child(status_panel(
                        StatusTone::Loading,
                        "EPISODE",
                        "Loading episode information",
                        "Fetching episode artwork, synopsis, and metadata.",
                    ));
                }
                if let Some(error) = d.episode_error.as_deref() {
                    section = section.child(status_panel(
                        StatusTone::Error,
                        "EPISODE UNAVAILABLE",
                        "Could not load this episode",
                        error,
                    ));
                }
                if let Some(episode) = &d.selected_episode {
                    let mut selected_episode = div().w_full().flex().flex_col().gap_2().child(
                        div()
                            .text_lg()
                            .text_color(rgb(Theme::TEXT_PRIMARY))
                            .child(format!(
                                "S{:02} E{:02} · {}",
                                episode.season_number, episode.episode_number, episode.name
                            )),
                    );
                    if let Some(still) = d.selected_episode_artwork.clone() {
                        selected_episode = selected_episode.child(
                            div()
                                .w_full()
                                .h(px(260.0))
                                .rounded_lg()
                                .overflow_hidden()
                                .child(img(still).w_full().h_full()),
                        );
                    }
                    let mut episode_facts = Vec::new();
                    if let Some(date) = episode.air_date.as_deref() {
                        episode_facts.push(date.to_owned());
                    }
                    if let Some(runtime) = episode.runtime {
                        episode_facts.push(format!("{runtime} min"));
                    }
                    if let Some(rating) = episode.vote_average {
                        episode_facts.push(format!("★ {rating:.1}/10"));
                    }
                    if !episode_facts.is_empty() {
                        selected_episode = selected_episode.child(
                            div()
                                .text_xs()
                                .text_color(rgb(Theme::TEXT_MUTED))
                                .child(episode_facts.join(" · ")),
                        );
                    }
                    let watched_item = WatchedItem {
                        profile_id: "default".to_owned(),
                        media_id: d.id.to_string(),
                        media_kind: MediaKind::Series,
                        season_number: Some(episode.season_number),
                        episode_number: Some(episode.episode_number),
                        marked_at_ms: 0,
                    };
                    let is_watched = self.playback_progress_repository
                        .is_watched(&watched_item)
                        .unwrap_or(false);
                    selected_episode = selected_episode.child(
                        div()
                            .id("toggle-episode-watched")
                            .cursor_pointer()
                            .rounded_md()
                            .bg(rgb(if is_watched { Theme::SURFACE_RAISED } else { Theme::ACCENT_DARK }))
                            .px_3()
                            .py_2()
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_selected_episode_watched(cx)))
                            .child(div().text_xs().text_color(rgb(if is_watched { Theme::TEXT_PRIMARY } else { Theme::ACCENT }))
                                .child(if is_watched { "MARK UNWATCHED" } else { "MARK WATCHED" })),
                    );
                    selected_episode = selected_episode.child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child(
                                episode
                                    .overview
                                    .as_deref()
                                    .filter(|value| !value.trim().is_empty())
                                    .unwrap_or("No episode synopsis is available.")
                                    .to_owned(),
                            ),
                    );
                    section = section.child(selected_episode);
                }
            }
            page = page.child(section);
        }
        page = page.child(self.render_source_section(d, cx));
        page
    }

    fn set_detail_source_sort(&mut self, sort: SourceSort, cx: &mut Context<Self>) {
        if let Some(details) = self.details.as_mut() {
            details.source_sort = sort;
            cx.notify();
        }
    }

    fn set_detail_source_filter(&mut self, filter: &str, enabled: bool, cx: &mut Context<Self>) {
        if let Some(details) = self.details.as_mut() {
            match filter {
                "direct" => details.source_show_direct = enabled,
                "torrent" => details.source_show_torrents = enabled,
                "external" => details.source_show_external = enabled,
                _ => return,
            }
            cx.notify();
        }
    }

    fn render_source_section(&self, details: &DetailsPage, cx: &mut Context<Self>) -> gpui::Div {
        let season = details.source_season;
        let episode = details.source_episode;
        let mut panel = div().w_full().flex().flex_col().gap_3().child(
            div()
                .text_lg()
                .text_color(rgb(Theme::TEXT_PRIMARY))
                .child("Sources"),
        );
        let action_label = if details.source_loading {
            "SEARCHING PROVIDERS…"
        } else if details.source_result.is_some() {
            "REFRESH SOURCES"
        } else {
            "FIND SOURCES"
        };
        panel = panel.child(
            div()
                .id("details-find-sources")
                .tab_index(90)
                .cursor_pointer()
                .rounded_md()
                .bg(rgb(Theme::ACCENT_DARK))
                .px_3()
                .py_2()
                .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                .aria_label("Find playback sources for this title or episode")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.load_detail_sources(season, episode, true, cx)
                }))
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(Theme::ACCENT))
                        .child(action_label),
                ),
        );
        if details.source_loading {
            return panel.child(status_panel(StatusTone::Loading, "SOURCE DISCOVERY", "Searching installed providers", "SENZO queries independent providers concurrently and keeps the interface responsive."));
        }
        if let Some(error) = details.source_error.as_deref() {
            return panel.child(status_panel(
                StatusTone::Error,
                "SOURCE DISCOVERY FAILED",
                "Could not load sources",
                error,
            ));
        }
        let Some(result) = details.source_result.as_ref() else {
            return panel.child(status_panel(StatusTone::Info, "SOURCE DISCOVERY", "Sources are ready to search", "Find sources to query your enabled Stremio add-ons, Nuvio scrapers and connected media servers."));
        };
        let count_line = format!(
            "{} source(s) from {} provider group(s){}",
            result.total_sources,
            result.groups.len(),
            if result.used_stale_cache {
                " · showing last successful results"
            } else {
                ""
            }
        );
        panel = panel.child(
            div()
                .text_xs()
                .text_color(rgb(Theme::TEXT_MUTED))
                .child(count_line),
        );
        let sorts = [
            (SourceSort::Recommended, "Recommended", "recommended"),
            (SourceSort::Quality, "Quality", "quality"),
            (SourceSort::Seeders, "Seeders", "seeders"),
            (SourceSort::Size, "Size", "size"),
        ];
        let mut sort_row = div().flex().flex_wrap().gap_2().items_center().child(
            div()
                .text_xs()
                .text_color(rgb(Theme::TEXT_MUTED))
                .child("SORT"),
        );
        for (sort, label, key) in sorts {
            let selected = details.source_sort == sort;
            sort_row = sort_row.child(
                div()
                    .id(format!("source-sort-{key}"))
                    .cursor_pointer()
                    .rounded_md()
                    .bg(rgb(if selected {
                        Theme::ACCENT_DARK
                    } else {
                        Theme::SURFACE
                    }))
                    .px_2()
                    .py_1()
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.set_detail_source_sort(sort, cx)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(if selected {
                                Theme::ACCENT
                            } else {
                                Theme::TEXT_SECONDARY
                            }))
                            .child(label),
                    ),
            );
        }
        panel = panel.child(sort_row);
        let filter_specs = [
            ("direct", "Direct", details.source_show_direct),
            ("torrent", "Torrents", details.source_show_torrents),
            ("external", "External", details.source_show_external),
        ];
        let mut filter_row = div().flex().flex_wrap().gap_2().items_center().child(
            div()
                .text_xs()
                .text_color(rgb(Theme::TEXT_MUTED))
                .child("SHOW"),
        );
        for (key, label, active) in filter_specs {
            filter_row = filter_row.child(
                div()
                    .id(format!("source-filter-{key}"))
                    .cursor_pointer()
                    .rounded_md()
                    .bg(rgb(if active {
                        Theme::ACCENT_DARK
                    } else {
                        Theme::SURFACE
                    }))
                    .px_2()
                    .py_1()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_detail_source_filter(key, !active, cx)
                    }))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(if active {
                                Theme::ACCENT
                            } else {
                                Theme::TEXT_MUTED
                            }))
                            .child(label),
                    ),
            );
        }
        panel = panel.child(filter_row);
        if result.groups.is_empty() {
            panel = panel.child(status_panel(StatusTone::Empty, "NO SOURCES FOUND", "No provider returned a playable source", "Check that add-ons and scrapers are enabled, network permission is granted, or a media server is connected."));
        }
        for group in &result.groups {
            let mut visible_sources = group
                .sources
                .iter()
                .filter(|source| {
                    (source.is_direct() && details.source_show_direct)
                        || (source.is_torrent() && details.source_show_torrents)
                        || (source.is_external() && details.source_show_external)
                })
                .cloned()
                .collect::<Vec<_>>();
            sort_sources(&mut visible_sources, details.source_sort);
            let mut section = div()
                .w_full()
                .flex()
                .flex_col()
                .gap_2()
                .rounded_md()
                .border_1()
                .border_color(rgb(Theme::BORDER))
                .bg(rgb(Theme::SURFACE))
                .p_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(Theme::TEXT_PRIMARY))
                                .child(group.provider_name.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(Theme::TEXT_MUTED))
                                .child(format!("{} source(s)", visible_sources.len())),
                        ),
                );
            if let Some(error) = group.error.as_deref() {
                section = section.child(
                    div()
                        .text_xs()
                        .text_color(rgb(Theme::DANGER))
                        .child(error.to_owned()),
                );
            }
            for source in &visible_sources {
                let target = match &source.target {
                    SourceTarget::Http(_) => "DIRECT HTTP",
                    SourceTarget::Torrent { .. } => "TORRENT",
                    SourceTarget::External(_) => "EXTERNAL",
                    SourceTarget::Youtube(_) => "YOUTUBE",
                };
                let quality = source.quality.as_deref().unwrap_or("Quality unknown");
                let mut metadata = vec![target.to_owned(), quality.to_owned()];
                if let Some(language) = source.language.as_deref() {
                    metadata.push(language.to_owned());
                }
                if let Some(seeders) = source.seeders {
                    metadata.push(format!("{seeders} seeders"));
                }
                if let Some(size) = source.size_bytes {
                    metadata.push(if size >= 1_000_000_000 {
                        format!("{:.1} GB", size as f64 / 1_000_000_000.0)
                    } else {
                        format!("{:.0} MB", size as f64 / 1_000_000.0)
                    });
                }
                let recommended =
                    result.recommended_source_id.as_deref() == Some(source.id.as_str());
                let mut card = div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .rounded_md()
                    .bg(rgb(Theme::SURFACE_RAISED))
                    .p_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(Theme::TEXT_PRIMARY))
                                    .child(source.title.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(if recommended {
                                        Theme::ACCENT
                                    } else {
                                        Theme::TEXT_MUTED
                                    }))
                                    .child(if recommended { "RECOMMENDED" } else { "" }),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(Theme::TEXT_MUTED))
                            .child(metadata.join(" · ")),
                    );
                if !source.badges.is_empty() {
                    card = card.child(
                        div().text_xs().text_color(rgb(Theme::ACCENT)).child(
                            source
                                .badges
                                .iter()
                                .map(|badge| badge.label.as_str())
                                .collect::<Vec<_>>()
                                .join(" · "),
                        ),
                    );
                }
                if let SourceTarget::Http(url) = &source.target {
                    let playback_url = url.clone();
                    let playback_headers = source.headers.clone();
                    card = card.child(
                        div()
                            .id(format!("play-source-{}", source.id))
                            .cursor_pointer()
                            .rounded_md()
                            .bg(rgb(Theme::ACCENT_DARK))
                            .px_2()
                            .py_2()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.play_source(
                                    playback_url.clone(),
                                    playback_headers.clone(),
                                    cx,
                                );
                            }))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(Theme::ACCENT))
                                    .child("PLAY IN MPV"),
                            ),
                    );
                }
                if source.is_torrent()
                    && let Ok(accounts) = self.debrid_manager.accounts("default")
                {
                    for account in accounts.into_iter().filter(|account| account.configured) {
                        let provider = account.provider;
                        let selected_source = source.clone();
                        card = card.child(
                            div()
                                .id(format!("resolve-{}-{}", source.id, provider.id()))
                                .cursor_pointer()
                                .rounded_md()
                                .bg(rgb(Theme::ACCENT_DARK))
                                .px_2()
                                .py_2()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.resolve_detail_torrent(
                                        selected_source.clone(),
                                        provider,
                                        cx,
                                    );
                                }))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(Theme::ACCENT))
                                        .child(format!("Resolve with {}", provider.name())),
                                ),
                        );
                    }
                }
                section = section.child(card);
            }
            panel = panel.child(section);
        }
        if details.resolver_loading {
            panel = panel.child(status_panel(
                StatusTone::Loading,
                "DEBRID RESOLUTION",
                "Resolving torrent",
                "The selected provider is preparing a direct source.",
            ));
        }
        if let Some(error) = details.resolver_error.as_deref() {
            panel = panel.child(status_panel(
                StatusTone::Error,
                "DEBRID RESOLUTION FAILED",
                "Could not resolve this torrent",
                error,
            ));
        }
        if let Some(resolved) = details.resolved_debrid_source.as_ref() {
            panel = panel.child(status_panel(
                StatusTone::Info,
                "DIRECT SOURCE READY",
                &format!("Resolved by {}", resolved.provider.name()),
                "The selected source is ready for native MPV.",
            ));
            panel = panel.child(
                div()
                    .text_xs()
                    .text_color(rgb(Theme::TEXT_MUTED))
                    .child(format!(
                        "{} · {}",
                        resolved.title,
                        details
                            .resolver_source_title
                            .as_deref()
                            .unwrap_or("Selected torrent")
                    )),
            );
            let resolved_url = resolved.url.clone();
            let resolved_headers = resolved.headers.clone();
            panel = panel.child(
                div()
                    .id("play-resolved-torrent")
                    .cursor_pointer()
                    .rounded_md()
                    .bg(rgb(Theme::ACCENT_DARK))
                    .px_3()
                    .py_2()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.play_source(resolved_url.clone(), resolved_headers.clone(), cx);
                    }))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::ACCENT))
                            .child("PLAY RESOLVED SOURCE"),
                    ),
            );
        }
        if self.playback_loading {
            panel = panel.child(status_panel(
                StatusTone::Loading,
                "NATIVE PLAYBACK",
                "Starting MPV",
                "Initializing the native playback engine on its dedicated worker thread.",
            ));
        }
        if let Some(error) = self.playback_error.as_deref() {
            panel = panel.child(status_panel(
                StatusTone::Error,
                "NATIVE PLAYBACK",
                "Could not start MPV",
                error,
            ));
        }
        if let Some(player) = self.mpv_player.as_ref() {
            let snapshot = player.snapshot();
            let status = match snapshot.status {
                PlaybackStatus::Idle => "Idle",
                PlaybackStatus::Loading => "Loading",
                PlaybackStatus::Playing => "Playing",
                PlaybackStatus::Paused => "Paused",
                PlaybackStatus::Stopped => "Stopped",
                PlaybackStatus::Ended => "Ended",
                PlaybackStatus::Error => "Error",
            };
            let position = snapshot.position_seconds.unwrap_or(0.0);
            let duration = snapshot.duration_seconds.unwrap_or(0.0);
            panel = panel.child(
                div()
                    .text_xs()
                    .text_color(rgb(Theme::TEXT_MUTED))
                    .child(format!(
                        "MPV: {status} · {:.0}/{:.0}s · volume {:.0}% · speed {:.2}x",
                        position, duration, snapshot.volume_percent, snapshot.speed
                    )),
            );
            let controls = [
                ("pause", if snapshot.paused { "PLAY" } else { "PAUSE" }),
                ("back", "−10s"),
                ("forward", "+10s"),
                ("volume-down", "VOL−"),
                ("volume-up", "VOL+"),
                ("mute", if snapshot.muted { "UNMUTE" } else { "MUTE" }),
                (
                    "speed",
                    if (snapshot.speed - 1.0).abs() < 0.01 {
                        "1.25×"
                    } else {
                        "1.0×"
                    },
                ),
            ];
            let mut control_row = div().flex().flex_wrap().gap_2().items_center();
            for (action, label) in controls {
                control_row = control_row.child(
                    div()
                        .id(format!("mpv-control-{action}"))
                        .cursor_pointer()
                        .rounded_md()
                        .bg(rgb(Theme::SURFACE_RAISED))
                        .px_2()
                        .py_2()
                        .on_click(cx.listener(move |this, _, _, cx| match action {
                            "pause" => this.toggle_pause_playback(cx),
                            "back" => this.seek_playback(-10.0, cx),
                            "forward" => this.seek_playback(10.0, cx),
                            "volume-down" => this.adjust_playback_volume(-5.0, cx),
                            "volume-up" => this.adjust_playback_volume(5.0, cx),
                            "mute" => this.toggle_playback_mute(cx),
                            "speed" => this.toggle_playback_speed(cx),
                            _ => {}
                        }))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(Theme::TEXT_SECONDARY))
                                .child(label),
                        ),
                );
            }
            panel = panel.child(control_row);

            let audio_tracks = snapshot
                .tracks
                .iter()
                .filter(|track| track.kind == TrackKind::Audio)
                .collect::<Vec<_>>();
            let subtitle_tracks = snapshot
                .tracks
                .iter()
                .filter(|track| track.kind == TrackKind::Subtitle)
                .collect::<Vec<_>>();
            for (kind, tracks) in [
                (TrackKind::Audio, audio_tracks),
                (TrackKind::Subtitle, subtitle_tracks),
            ] {
                if tracks.is_empty() {
                    continue;
                }
                panel = panel.child(div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child(
                    if kind == TrackKind::Audio {
                        "AUDIO TRACKS"
                    } else {
                        "SUBTITLE TRACKS"
                    },
                ));
                let mut track_row = div().flex().flex_wrap().gap_2().items_center();
                for track in tracks {
                    let id = track.id;
                    let selected = track.selected;
                    let text = format!(
                        "{}{}{}",
                        track.language.as_deref().unwrap_or("und"),
                        track
                            .title
                            .as_deref()
                            .map(|title| format!(" · {title}"))
                            .unwrap_or_default(),
                        if selected { " ✓" } else { "" }
                    );
                    track_row = track_row.child(
                        div()
                            .id(format!(
                                "mpv-track-{}-{id}",
                                if kind == TrackKind::Audio {
                                    "audio"
                                } else {
                                    "subtitle"
                                }
                            ))
                            .cursor_pointer()
                            .rounded_md()
                            .bg(rgb(if selected {
                                Theme::ACCENT_DARK
                            } else {
                                Theme::SURFACE_RAISED
                            }))
                            .px_2()
                            .py_1()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_playback_track(kind, id, cx)
                            }))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(if selected {
                                        Theme::ACCENT
                                    } else {
                                        Theme::TEXT_SECONDARY
                                    }))
                                    .child(text),
                            ),
                    );
                }
                panel = panel.child(track_row);
            }

            panel = panel.child(
                div()
                    .id("stop-native-playback")
                    .cursor_pointer()
                    .rounded_md()
                    .bg(rgb(Theme::SURFACE_RAISED))
                    .px_3()
                    .py_2()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.persist_playback_snapshot(true, cx);
                        this.stop_playback();
                        this.start_playback_refresh(cx);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child("STOP PLAYBACK"),
                    ),
            );
        }
        let failed = result
            .diagnostics
            .iter()
            .filter(|item| item.error.is_some())
            .count();
        if failed > 0 {
            panel = panel.child(div().text_xs().text_color(rgb(Theme::TEXT_MUTED))
                .child(format!("{failed} provider adapter(s) reported an error. Successful providers remain available.")));
        }
        panel
    }

    fn render_player_screen(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let snapshot = self
            .mpv_player
            .as_ref()
            .map(MpvPlayer::snapshot)
            .unwrap_or_default();
        let duration = snapshot.duration_seconds.unwrap_or(0.0).max(0.0);
        let position = snapshot
            .position_seconds
            .unwrap_or(0.0)
            .clamp(0.0, duration);
        let percent = if duration > 0.0 {
            (position / duration * 100.0) as f32
        } else {
            0.0
        };
        if !self.playback_scrubbing {
            let current = self.playback_timeline.read(cx).value().end();
            if (current - percent).abs() > 0.5 {
                let timeline = self.playback_timeline.clone();
                timeline.update(cx, |state, cx| {
                    state.set_value(percent, window, cx);
                });
            }
        }

        let status = if self.playback_loading {
            "CONNECTING"
        } else if snapshot.buffering && snapshot.status == PlaybackStatus::Playing {
            "BUFFERING"
        } else {
            match snapshot.status {
                PlaybackStatus::Idle => "READY",
                PlaybackStatus::Loading => "LOADING",
                PlaybackStatus::Playing => "PLAYING",
                PlaybackStatus::Paused => "PAUSED",
                PlaybackStatus::Stopped => "STOPPED",
                PlaybackStatus::Ended => "ENDED",
                PlaybackStatus::Error => "PLAYBACK ERROR",
            }
        };
        let is_paused = snapshot.paused || snapshot.status == PlaybackStatus::Ended;
        let title = self
            .details
            .as_ref()
            .map(|details| details.title.as_str())
            .unwrap_or("Now Playing");
        let source_title = self
            .playback_sources
            .get(self.playback_source_index)
            .map(|source| source.title.as_str())
            .unwrap_or("Direct source");
        let stream_title = snapshot
            .media_title
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(source_title);

        let mut root = div()
            .id("immersive-player-screen")
            .key_context("SENZO_PLAYER")
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .justify_between()
            .bg(rgb(0x050607))
            .text_color(rgb(Theme::TEXT_PRIMARY))
            .on_mouse_move(cx.listener(|this, _event, _window, cx| {
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerTogglePlayback, _window, cx| {
                this.toggle_pause_playback(cx);
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerSeekBack, _window, cx| {
                this.seek_playback(-10.0, cx);
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerSeekForward, _window, cx| {
                this.seek_playback(10.0, cx);
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerVolumeDown, _window, cx| {
                this.adjust_playback_volume(-5.0, cx);
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerVolumeUp, _window, cx| {
                this.adjust_playback_volume(5.0, cx);
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerToggleMute, _window, cx| {
                this.toggle_playback_mute(cx);
                this.reveal_playback_controls(cx);
            }))
            .on_action(
                cx.listener(|this, _: &PlayerToggleFullscreen, _window, cx| {
                    this.toggle_player_fullscreen(cx);
                    this.reveal_playback_controls(cx);
                }),
            )
            .on_action(cx.listener(|this, _: &PlayerCycleResize, _window, cx| {
                this.cycle_player_resize_mode(cx);
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerShowAudio, _window, cx| {
                this.playback_panel = Some(PlayerPanel::Audio);
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerShowSubtitles, _window, cx| {
                this.playback_panel = Some(PlayerPanel::Subtitles);
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerShowChapters, _window, cx| {
                this.playback_panel = Some(PlayerPanel::Chapters);
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerShowInfo, _window, cx| {
                this.playback_panel = Some(PlayerPanel::StreamInfo);
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerShowSources, _window, cx| {
                this.playback_panel = Some(PlayerPanel::Sources);
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerRetry, _window, cx| {
                this.retry_playback(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerNextSource, _window, cx| {
                this.try_next_playback_source(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerPreviousEpisode, _window, cx| {
                this.navigate_selected_episode(-1, true, cx);
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerNextEpisode, _window, cx| {
                this.navigate_selected_episode(1, true, cx);
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerTogglePip, _window, cx| {
                this.toggle_player_picture_in_picture(cx);
                this.reveal_playback_controls(cx);
            }));

        if self.playback_controls_visible {
            let mut header = div()
                .w_full()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_3()
                .bg(rgb(0x101418))
                .px_5()
                .py_3()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(Theme::ACCENT))
                                .child("SENZO · NATIVE PLAYER"),
                        )
                        .child(
                            div()
                                .text_lg()
                                .text_color(rgb(Theme::TEXT_PRIMARY))
                                .child(title.to_owned()),
                        ),
                );
            let panel_specs = [
                (PlayerPanel::Audio, "AUDIO"),
                (PlayerPanel::Subtitles, "SUBTITLES"),
                (PlayerPanel::Chapters, "CHAPTERS"),
                (PlayerPanel::StreamInfo, "INFO"),
                (PlayerPanel::Sources, "SOURCES"),
            ];
            let mut nav = div().flex().flex_wrap().items_center().gap_2();
            for (panel, label) in panel_specs {
                let selected = self.playback_panel == Some(panel);
                nav = nav.child(
                    div()
                        .id(format!("player-panel-{}", label.to_ascii_lowercase()))
                        .cursor_pointer()
                        .rounded_md()
                        .bg(rgb(if selected {
                            Theme::ACCENT_DARK
                        } else {
                            Theme::SURFACE_RAISED
                        }))
                        .px_3()
                        .py_2()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.playback_panel = if this.playback_panel == Some(panel) {
                                None
                            } else {
                                Some(panel)
                            };
                            this.reveal_playback_controls(cx);
                        }))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(if selected {
                                    Theme::ACCENT
                                } else {
                                    Theme::TEXT_SECONDARY
                                }))
                                .child(label),
                        ),
                );
            }
            nav = nav.child(
                div()
                    .id("player-previous-episode")
                    .cursor_pointer()
                    .rounded_md()
                    .bg(rgb(Theme::SURFACE_RAISED))
                    .px_3()
                    .py_2()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.navigate_selected_episode(-1, true, cx);
                        this.reveal_playback_controls(cx);
                    }))
                    .child(div().text_xs().text_color(rgb(Theme::TEXT_SECONDARY)).child("PREV EP")),
            );
            nav = nav.child(
                div()
                    .id("player-next-episode")
                    .cursor_pointer()
                    .rounded_md()
                    .bg(rgb(Theme::SURFACE_RAISED))
                    .px_3()
                    .py_2()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.navigate_selected_episode(1, true, cx);
                        this.reveal_playback_controls(cx);
                    }))
                    .child(div().text_xs().text_color(rgb(Theme::TEXT_SECONDARY)).child("NEXT EP")),
            );
            nav = nav.child(
                div()
                    .id("player-autoplay-next")
                    .cursor_pointer()
                    .rounded_md()
                    .bg(rgb(if self.playback_autoplay_next { Theme::ACCENT_DARK } else { Theme::SURFACE_RAISED }))
                    .px_3()
                    .py_2()
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_autoplay_next(cx)))
                    .child(div().text_xs().text_color(rgb(if self.playback_autoplay_next { Theme::ACCENT } else { Theme::TEXT_SECONDARY }))
                        .child(if self.playback_autoplay_next { "AUTO NEXT ON" } else { "AUTO NEXT OFF" })),
            );
            nav = nav.child(
                div()
                    .id("player-resize-mode")
                    .cursor_pointer()
                    .rounded_md()
                    .bg(rgb(Theme::SURFACE_RAISED))
                    .px_3()
                    .py_2()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cycle_player_resize_mode(cx);
                        this.reveal_playback_controls(cx);
                    }))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child(self.playback_resize_label()),
                    ),
            );
            nav = nav.child(
                div()
                    .id("player-pip")
                    .cursor_pointer()
                    .rounded_md()
                    .bg(rgb(Theme::SURFACE_RAISED))
                    .px_3()
                    .py_2()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_player_picture_in_picture(cx);
                        this.reveal_playback_controls(cx);
                    }))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child(if self.playback_picture_in_picture {
                                "RESTORE"
                            } else {
                                "PIP"
                            }),
                    ),
            );
            nav = nav.child(
                div()
                    .id("player-fullscreen")
                    .cursor_pointer()
                    .rounded_md()
                    .bg(rgb(Theme::SURFACE_RAISED))
                    .px_3()
                    .py_2()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_player_fullscreen(cx);
                        this.reveal_playback_controls(cx);
                    }))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child(if self.playback_fullscreen {
                                "WINDOWED"
                            } else {
                                "FULLSCREEN"
                            }),
                    ),
            );
            nav = nav.child(
                div()
                    .id("player-close")
                    .cursor_pointer()
                    .rounded_md()
                    .bg(rgb(Theme::DANGER))
                    .px_3()
                    .py_2()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.close_player(window, cx);
                    }))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(Theme::TEXT_PRIMARY))
                            .child("CLOSE"),
                    ),
            );
            header = header.child(nav);
            root = root.child(header);
        }

        let mut stage = div()
            .flex_1()
            .w_full()
            .min_h(px(0.0))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w_full()
                    .max_w(px(840.0))
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(Theme::BORDER))
                    .bg(rgb(0x101418))
                    .p_5()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(Theme::ACCENT))
                            .child("NATIVE VIDEO OUTPUT"),
                    )
                    .child(
                        div()
                            .text_2xl()
                            .text_color(rgb(Theme::TEXT_PRIMARY))
                            .child(stream_title.to_owned()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child(format!(
                                "{} · {} · {}",
                                status,
                                snapshot.video_codec.as_deref().unwrap_or("Video pending"),
                                match (snapshot.video_width, snapshot.video_height) {
                                    (Some(width), Some(height)) => format!("{width}×{height}"),
                                    _ => "Waiting for video dimensions".to_owned(),
                                }
                            )),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_MUTED))
                            .child("MPV renders the picture in its own native window. SENZO keeps playback controls in this GPUI player."),
                    )
                    .child(
                        div()
                            .w_full()
                            .text_xs()
                            .text_color(rgb(Theme::TEXT_MUTED))
                            .child(if snapshot.buffering {
                                format!("BUFFERING · {:.0}% CACHED", snapshot.buffered_percent)
                            } else {
                                format!(
                                    "{} · VOLUME {:.0}% · SPEED {:.2}×",
                                    status, snapshot.volume_percent, snapshot.speed
                                )
                            }),
                    ),
            );
        if self.playback_loading {
            stage = stage.child(status_panel(
                StatusTone::Loading,
                "STARTING VIDEO",
                "Opening native MPV output",
                "The selected source is loading on the dedicated playback worker.",
            ));
        }
        if snapshot.status == PlaybackStatus::Error || self.playback_error.is_some() {
            stage = stage.child(status_panel(
                StatusTone::Error,
                "PLAYBACK FAILED",
                "The selected source could not continue",
                self.playback_error
                    .as_deref()
                    .unwrap_or("The native player reported a playback error."),
            ));
            stage = stage.child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .id("player-retry")
                            .cursor_pointer()
                            .rounded_md()
                            .bg(rgb(Theme::ACCENT_DARK))
                            .px_3()
                            .py_2()
                            .on_click(cx.listener(|this, _, _, cx| this.retry_playback(cx)))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(Theme::ACCENT))
                                    .child("RETRY"),
                            ),
                    )
                    .child(
                        div()
                            .id("player-next-source")
                            .cursor_pointer()
                            .rounded_md()
                            .bg(rgb(Theme::SURFACE_RAISED))
                            .px_3()
                            .py_2()
                            .on_click(
                                cx.listener(|this, _, _, cx| this.try_next_playback_source(cx)),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(Theme::TEXT_SECONDARY))
                                    .child("TRY NEXT SOURCE"),
                            ),
                    ),
            );
        }
        root = root.child(stage);

        if let Some(progress) = self.playback_resume_prompt.as_ref() {
            let episode_label = match (progress.season_number, progress.episode_number) {
                (Some(season), Some(episode)) => format!("S{season:02} E{episode:02}"),
                _ => "MOVIE".to_owned(),
            };
            let saved_time = format_playback_time(progress.position_ms as f64 / 1000.0);
            root = root.child(
                div()
                    .id("player-resume-prompt")
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgb(0x050607).opacity(0.94))
                    .child(
                        div()
                            .w(px(520.0))
                            .rounded_lg()
                            .border_1()
                            .border_color(rgb(Theme::BORDER_STRONG))
                            .bg(rgb(Theme::SURFACE))
                            .p_6()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(div().text_xs().text_color(rgb(Theme::ACCENT)).child("CONTINUE WATCHING"))
                            .child(div().text_2xl().text_color(rgb(Theme::TEXT_PRIMARY)).child(progress.title.clone()))
                            .child(div().text_sm().text_color(rgb(Theme::TEXT_SECONDARY)).child(format!("{episode_label} · Last position {saved_time}")))
                            .child(div().flex().gap_3()
                                .child(div().id("resume-playback")
                                    .cursor_pointer()
                                    .rounded_md()
                                    .bg(rgb(Theme::ACCENT))
                                    .px_4()
                                    .py_3()
                                    .on_click(cx.listener(|this, _, _, cx| this.choose_resume_action(true, cx)))
                                    .child(div().text_sm().text_color(rgb(Theme::BACKGROUND)).child("RESUME")))
                                .child(div().id("restart-playback")
                                    .cursor_pointer()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(rgb(Theme::BORDER_STRONG))
                                    .bg(rgb(Theme::SURFACE_RAISED))
                                    .px_4()
                                    .py_3()
                                    .on_click(cx.listener(|this, _, _, cx| this.choose_resume_action(false, cx)))
                                    .child(div().text_sm().text_color(rgb(Theme::TEXT_PRIMARY)).child("START OVER")))),
                    ),
            );
        }

        if let Some((label, target_seconds)) = chapter_skip_target(&snapshot) {
            root = root.child(
                div()
                    .id("player-skip-named-chapter")
                    .absolute()
                    .bottom(px(112.0))
                    .right(px(32.0))
                    .cursor_pointer()
                    .rounded_md()
                    .bg(rgb(Theme::ACCENT))
                    .px_4()
                    .py_3()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.seek_playback_absolute(target_seconds, cx);
                        this.reveal_playback_controls(cx);
                    }))
                    .child(div().text_sm().text_color(rgb(Theme::BACKGROUND)).child(label)),
            );
        }

        if let Some(panel) = self.playback_panel {
            let mut body = div()
                .w_full()
                .max_h(px(300.0))
                .overflow_y_scrollbar()
                .bg(rgb(0x11171B))
                .border_1()
                .border_color(rgb(Theme::BORDER))
                .p_4()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(div().text_lg().text_color(rgb(Theme::TEXT_PRIMARY)).child(
                            match panel {
                                PlayerPanel::Audio => "Audio tracks",
                                PlayerPanel::Subtitles => "Subtitles",
                                PlayerPanel::Chapters => "Chapters",
                                PlayerPanel::StreamInfo => "Stream information",
                                PlayerPanel::Sources => "Playback sources",
                            },
                        ))
                        .child(
                            div()
                                .id("player-panel-close")
                                .cursor_pointer()
                                .rounded_md()
                                .bg(rgb(Theme::SURFACE_RAISED))
                                .px_3()
                                .py_1()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.playback_panel = None;
                                    this.reveal_playback_controls(cx);
                                }))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(Theme::TEXT_SECONDARY))
                                        .child("CLOSE"),
                                ),
                        ),
                );

            match panel {
                PlayerPanel::Audio => {
                    let tracks = snapshot
                        .tracks
                        .iter()
                        .filter(|track| track.kind == TrackKind::Audio)
                        .collect::<Vec<_>>();
                    if tracks.is_empty() {
                        body = body.child(status_panel(
                            StatusTone::Info,
                            "AUDIO",
                            "No audio tracks reported",
                            "The active source has not exposed audio-track metadata.",
                        ));
                    }
                    for track in tracks {
                        let id = track.id;
                        let selected = track.selected;
                        let label = format!(
                            "{}{}{}",
                            track.language.as_deref().unwrap_or("Unknown language"),
                            track
                                .title
                                .as_deref()
                                .map(|value| format!(" · {value}"))
                                .unwrap_or_default(),
                            track
                                .codec
                                .as_deref()
                                .map(|value| format!(" · {value}"))
                                .unwrap_or_default()
                        );
                        body = body.child(
                            div()
                                .id(format!("player-audio-track-{id}"))
                                .cursor_pointer()
                                .rounded_md()
                                .bg(rgb(if selected {
                                    Theme::ACCENT_DARK
                                } else {
                                    Theme::SURFACE_RAISED
                                }))
                                .px_3()
                                .py_2()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.select_playback_track(TrackKind::Audio, id, cx);
                                    this.reveal_playback_controls(cx);
                                }))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(if selected {
                                            Theme::ACCENT
                                        } else {
                                            Theme::TEXT_SECONDARY
                                        }))
                                        .child(label),
                                ),
                        );
                    }
                }
                PlayerPanel::Subtitles => {
                    let tracks = snapshot
                        .tracks
                        .iter()
                        .filter(|track| track.kind == TrackKind::Subtitle)
                        .collect::<Vec<_>>();
                    let mut row = div().flex().flex_wrap().items_center().gap_2();
                    let subtitle_actions = [
                        ("player-subtitles-none", "NONE", "none"),
                        (
                            "subtitle-visible",
                            if self.playback_subtitles_visible {
                                "HIDE"
                            } else {
                                "SHOW"
                            },
                            "visible",
                        ),
                        ("subtitle-size-down", "A−", "smaller"),
                        ("subtitle-size-up", "A+", "larger"),
                        ("subtitle-delay-down", "DELAY −0.25s", "delay-down"),
                        ("subtitle-delay-up", "DELAY +0.25s", "delay-up"),
                    ];
                    for (id, label, action) in subtitle_actions {
                        row = row.child(
                            div()
                                .id(id)
                                .cursor_pointer()
                                .rounded_md()
                                .bg(rgb(Theme::SURFACE_RAISED))
                                .px_3()
                                .py_2()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    match action {
                                        "none" => {
                                            this.run_player_command(
                                                |player| player.select_subtitle_track(None),
                                                cx,
                                            );
                                            this.playback_subtitles_visible = false;
                                        }
                                        "visible" => this.toggle_subtitle_visibility(cx),
                                        "smaller" => this.adjust_subtitle_font_size(-2, cx),
                                        "larger" => this.adjust_subtitle_font_size(2, cx),
                                        "delay-down" => this.adjust_subtitle_delay(-0.25, cx),
                                        "delay-up" => this.adjust_subtitle_delay(0.25, cx),
                                        _ => {}
                                    }
                                    this.reveal_playback_controls(cx);
                                }))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(Theme::TEXT_SECONDARY))
                                        .child(label),
                                ),
                        );
                    }
                    body = body.child(row);
                    body = body.child(div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child(
                        format!(
                            "Font size {} · subtitle delay {:.2}s",
                            self.playback_subtitle_font_size, self.playback_subtitle_delay_seconds
                        ),
                    ));
                    for track in tracks {
                        let id = track.id;
                        let selected = track.selected;
                        let label = format!(
                            "{}{}{}",
                            track.language.as_deref().unwrap_or("Unknown language"),
                            track
                                .title
                                .as_deref()
                                .map(|value| format!(" · {value}"))
                                .unwrap_or_default(),
                            track
                                .codec
                                .as_deref()
                                .map(|value| format!(" · {value}"))
                                .unwrap_or_default()
                        );
                        body = body.child(
                            div()
                                .id(format!("player-subtitle-track-{id}"))
                                .cursor_pointer()
                                .rounded_md()
                                .bg(rgb(if selected {
                                    Theme::ACCENT_DARK
                                } else {
                                    Theme::SURFACE_RAISED
                                }))
                                .px_3()
                                .py_2()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.select_playback_track(TrackKind::Subtitle, id, cx);
                                    this.playback_subtitles_visible = true;
                                    this.reveal_playback_controls(cx);
                                }))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(if selected {
                                            Theme::ACCENT
                                        } else {
                                            Theme::TEXT_SECONDARY
                                        }))
                                        .child(label),
                                ),
                        );
                    }
                    for subtitle in self.addon_subtitle_results.iter() {
                        let subtitle_url = subtitle.url.clone();
                        let label = format!(
                            "ADD-ON · {} · {}",
                            subtitle.language,
                            subtitle
                                .label
                                .as_deref()
                                .unwrap_or(subtitle.mime_type.as_deref().unwrap_or("Subtitle"))
                        );
                        let identity = subtitle.id.as_deref().unwrap_or(&subtitle.url).to_owned();
                        body = body.child(
                            div()
                                .id(format!("player-addon-subtitle-{identity}"))
                                .cursor_pointer()
                                .rounded_md()
                                .bg(rgb(Theme::SURFACE_RAISED))
                                .px_3()
                                .py_2()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let url = subtitle_url.clone();
                                    this.run_player_command(
                                        move |player| {
                                            player.add_external_subtitle(&url, &BTreeMap::new())
                                        },
                                        cx,
                                    );
                                    this.playback_subtitles_visible = true;
                                    this.reveal_playback_controls(cx);
                                }))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(Theme::TEXT_SECONDARY))
                                        .child(label),
                                ),
                        );
                    }
                    body = body.child(
                        Input::new(&self.subtitle_url_input)
                            .w_full()
                            .bg(rgb(Theme::SURFACE))
                            .border_1()
                            .border_color(rgb(Theme::BORDER))
                            .rounded_md(),
                    );
                    body = body.child(
                        div()
                            .id("player-add-external-subtitle")
                            .cursor_pointer()
                            .rounded_md()
                            .bg(rgb(Theme::ACCENT_DARK))
                            .px_3()
                            .py_2()
                            .on_click(cx.listener(|this, _, _, cx| this.add_external_subtitle(cx)))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(Theme::ACCENT))
                                    .child("ADD HTTP(S) SUBTITLE"),
                            ),
                    );
                    body = body.child(div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child("External subtitles can be added from a direct HTTP(S) URL, including an add-on subtitle URL. Appearance controls currently cover size, visibility and timing."));
                }
                PlayerPanel::Chapters => {
                    if snapshot.chapters.is_empty() {
                        body = body.child(status_panel(
                            StatusTone::Info,
                            "CHAPTERS",
                            "No chapter markers found",
                            "This media source did not expose chapter metadata.",
                        ));
                    }
                    for chapter in &snapshot.chapters {
                        let seconds = chapter.start_seconds;
                        let label = chapter
                            .title
                            .as_deref()
                            .filter(|value| !value.trim().is_empty())
                            .unwrap_or("Chapter");
                        body = body.child(
                            div()
                                .id(format!("player-chapter-{}", chapter.index))
                                .cursor_pointer()
                                .rounded_md()
                                .bg(rgb(Theme::SURFACE_RAISED))
                                .px_3()
                                .py_2()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.seek_playback_absolute(seconds, cx);
                                    this.playback_panel = None;
                                    this.reveal_playback_controls(cx);
                                }))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(Theme::TEXT_SECONDARY))
                                        .child(format!(
                                            "{} · {}",
                                            format_playback_time(seconds),
                                            label
                                        )),
                                ),
                        );
                    }
                }
                PlayerPanel::StreamInfo => {
                    let resolution = match (snapshot.video_width, snapshot.video_height) {
                        (Some(width), Some(height)) => format!("{width}×{height}"),
                        _ => "Not available".to_owned(),
                    };
                    body = body.child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child(format!("Source: {source_title}")),
                    );
                    body = body.child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child(format!("Resolution: {resolution}")),
                    );
                    body = body.child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child(format!(
                                "Video codec: {}",
                                snapshot.video_codec.as_deref().unwrap_or("Unknown")
                            )),
                    );
                    body = body.child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child(format!(
                                "Audio codec: {}",
                                snapshot.audio_codec.as_deref().unwrap_or("Unknown")
                            )),
                    );
                    body = body.child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child(format!(
                                "Audio tracks: {} · Subtitle tracks: {}",
                                snapshot
                                    .tracks
                                    .iter()
                                    .filter(|track| track.kind == TrackKind::Audio)
                                    .count(),
                                snapshot
                                    .tracks
                                    .iter()
                                    .filter(|track| track.kind == TrackKind::Subtitle)
                                    .count()
                            )),
                    );
                    if let Some(details) = self.details.as_ref() {
                        if let Some(release) = details.release_date.as_deref() {
                            body = body.child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(Theme::TEXT_SECONDARY))
                                    .child(format!("Release: {release}")),
                            );
                        }
                        if !details.genres.is_empty() {
                            body = body.child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(Theme::TEXT_SECONDARY))
                                    .child(format!("Genres: {}", details.genres.join(" · "))),
                            );
                        }
                        if let Some(overview) = details
                            .overview
                            .as_deref()
                            .filter(|value| !value.trim().is_empty())
                        {
                            body = body.child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(Theme::TEXT_SECONDARY))
                                    .child(overview.to_owned()),
                            );
                        }
                        if let Some(rating) = details.rating {
                            body = body.child(div().text_sm().text_color(rgb(Theme::TEXT_SECONDARY)).child(format!("Metadata rating: {rating:.1}/10 (not a content-age classification)")));
                        }
                    }
                    body = body.child(div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child("Content-age guidance is unavailable from the current metadata contract; SENZO does not infer a parental rating."));
                }
                PlayerPanel::Sources => {
                    body = body.child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child(format!(
                                "Source {}/{} · {source_title}",
                                self.playback_source_index + 1,
                                self.playback_sources.len().max(1)
                            )),
                    );
                    body = body.child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                div()
                                    .id("player-source-retry")
                                    .cursor_pointer()
                                    .rounded_md()
                                    .bg(rgb(Theme::ACCENT_DARK))
                                    .px_3()
                                    .py_2()
                                    .on_click(cx.listener(|this, _, _, cx| this.retry_playback(cx)))
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(rgb(Theme::ACCENT))
                                            .child("RETRY CURRENT SOURCE"),
                                    ),
                            )
                            .child(
                                div()
                                    .id("player-source-next")
                                    .cursor_pointer()
                                    .rounded_md()
                                    .bg(rgb(Theme::SURFACE_RAISED))
                                    .px_3()
                                    .py_2()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.try_next_playback_source(cx)
                                    }))
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(rgb(Theme::TEXT_SECONDARY))
                                            .child("TRY NEXT SOURCE"),
                                    ),
                            ),
                    );
                    for (index, source) in self.playback_sources.iter().enumerate() {
                        let source_index = index;
                        let url = source.url.clone();
                        let headers = source.headers.clone();
                        let label = format!(
                            "{}{}",
                            source.title,
                            if index == self.playback_source_index {
                                " · ACTIVE"
                            } else {
                                ""
                            }
                        );
                        body = body.child(
                            div()
                                .id(format!("player-source-{index}"))
                                .cursor_pointer()
                                .rounded_md()
                                .bg(rgb(if index == self.playback_source_index {
                                    Theme::ACCENT_DARK
                                } else {
                                    Theme::SURFACE_RAISED
                                }))
                                .px_3()
                                .py_2()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.playback_source_index = source_index;
                                    this.start_playback_url(url.clone(), headers.clone(), cx);
                                    this.reveal_playback_controls(cx);
                                }))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(if index == self.playback_source_index {
                                            Theme::ACCENT
                                        } else {
                                            Theme::TEXT_SECONDARY
                                        }))
                                        .child(label),
                                ),
                        );
                    }
                }
            }
            root = root.child(body);
        }

        if self.playback_controls_visible {
            let remaining = (duration - position).max(0.0);
            let mut footer = div()
                .w_full()
                .flex()
                .flex_col()
                .gap_3()
                .bg(rgb(0x101418))
                .px_5()
                .py_4()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(Theme::TEXT_MUTED))
                                .child(format_playback_time(position)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(Theme::TEXT_MUTED))
                                .child("TIMELINE"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(Theme::TEXT_MUTED))
                                .child(format!("-{}", format_playback_time(remaining))),
                        ),
                )
                .child(
                    div()
                        .id("player-playback-timeline")
                        .child(Slider::new(&self.playback_timeline).w_full().h(px(24.0))),
                );

            let mut controls = div()
                .w_full()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_center()
                .gap_2();
            controls = controls.child(
                div()
                    .id("player-seek-back")
                    .cursor_pointer()
                    .rounded_md()
                    .bg(rgb(Theme::SURFACE_RAISED))
                    .px_4()
                    .py_3()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.seek_playback(-10.0, cx);
                        this.reveal_playback_controls(cx);
                    }))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child("−10s"),
                    ),
            );
            controls = controls.child(
                div()
                    .id("player-play-pause")
                    .cursor_pointer()
                    .rounded_md()
                    .bg(rgb(Theme::ACCENT_DARK))
                    .px_4()
                    .py_3()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_pause_playback(cx);
                        this.reveal_playback_controls(cx);
                    }))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::ACCENT))
                            .child(if is_paused { "PLAY" } else { "PAUSE" }),
                    ),
            );
            controls = controls.child(
                div()
                    .id("player-seek-forward")
                    .cursor_pointer()
                    .rounded_md()
                    .bg(rgb(Theme::SURFACE_RAISED))
                    .px_4()
                    .py_3()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.seek_playback(10.0, cx);
                        this.reveal_playback_controls(cx);
                    }))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child("+10s"),
                    ),
            );
            controls = controls.child(
                div()
                    .id("player-volume-down")
                    .cursor_pointer()
                    .rounded_md()
                    .bg(rgb(Theme::SURFACE_RAISED))
                    .px_3()
                    .py_3()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.adjust_playback_volume(-5.0, cx);
                        this.reveal_playback_controls(cx);
                    }))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child("VOL −"),
                    ),
            );
            controls = controls.child(
                div()
                    .id("player-volume-up")
                    .cursor_pointer()
                    .rounded_md()
                    .bg(rgb(Theme::SURFACE_RAISED))
                    .px_3()
                    .py_3()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.adjust_playback_volume(5.0, cx);
                        this.reveal_playback_controls(cx);
                    }))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child("VOL +"),
                    ),
            );
            controls = controls.child(
                div()
                    .id("player-mute")
                    .cursor_pointer()
                    .rounded_md()
                    .bg(rgb(Theme::SURFACE_RAISED))
                    .px_3()
                    .py_3()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_playback_mute(cx);
                        this.reveal_playback_controls(cx);
                    }))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child(if snapshot.muted { "UNMUTE" } else { "MUTE" }),
                    ),
            );
            controls = controls.child(
                div()
                    .id("player-speed")
                    .cursor_pointer()
                    .rounded_md()
                    .bg(rgb(Theme::SURFACE_RAISED))
                    .px_3()
                    .py_3()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_playback_speed(cx);
                        this.reveal_playback_controls(cx);
                    }))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child(format!("{:.2}×", snapshot.speed)),
                    ),
            );
            footer = footer.child(controls);
            root = root.child(footer);
        } else {
            root = root.child(div().w_full().flex().justify_center().p_3().child(
                div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child(
                    "Move the pointer or press Space to reveal player controls · Esc to return",
                ),
            ));
        }

        root
    }

    fn playback_resize_label(&self) -> &'static str {
        match self.playback_resize_mode {
            ResizeMode::Fit => "FIT",
            ResizeMode::Fill => "FILL",
            ResizeMode::Stretch => "STRETCH",
            ResizeMode::Zoom => "ZOOM",
        }
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
        self.stop_playback();
        self.remember_current_page_scroll();
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

    fn close_person_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.person_details_cancellation.cancel();
        self.person_details = None;
        if let Some(previous) = self.detail_history.pop() {
            match previous {
                DetailRoute::Title {
                    id,
                    media_type,
                    title,
                } => self.load_details(id, media_type, title, cx),
                DetailRoute::Person { id, name, profile } => {
                    self.load_person_details(id, name, profile, cx)
                }
            }
            return;
        }
        let page = self.navigation.current();
        let focus_id = self.home_return_focus_id.take();
        self.restore_page_position(page, focus_id, window);
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
                        this.open_related_detail(id, media_type, media_title.clone(), cx);
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
                .aria_label(if self.detail_history.is_empty() {
                    "Back to search results"
                } else {
                    "Back to previous title"
                })
                .on_click(cx.listener(|this, _, window, cx| this.close_person_details(window, cx)))
                .child(div().text_sm().text_color(rgb(Theme::ACCENT)).child(
                    if self.detail_history.is_empty() {
                        "← Back to results"
                    } else {
                        "← Back to title"
                    },
                )),
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

    fn record_search_query(&mut self, query: &str) {
        let updated = record_search_history(&self.recent_searches, query);
        if updated == self.recent_searches {
            return;
        }
        self.recent_searches = updated;
        let key = SettingKey::new("search.recent").expect("valid search history key");
        match serde_json::to_string(&self.recent_searches) {
            Ok(payload) => {
                if let Err(error) = self.settings.set(&key, &payload) {
                    tracing::warn!(error = %error, "could not persist search history");
                }
            }
            Err(error) => tracing::warn!(error = %error, "could not serialize search history"),
        }
    }

    fn clear_search_history(&mut self) {
        self.recent_searches.clear();
        let key = SettingKey::new("search.recent").expect("valid search history key");
        if let Err(error) = self.settings.set(&key, "[]") {
            tracing::warn!(error = %error, "could not clear persisted search history");
        }
    }

    fn set_search_page(&mut self, page: u32, cx: &mut Context<Self>) {
        if page == 0 || page > self.search_total_pages.min(500) || page == self.search_page {
            return;
        }
        self.search_page = page;
        self.schedule_search(cx);
    }

    fn discovery_state(&self, media_type: MediaType) -> &DiscoveryPageState {
        match media_type {
            MediaType::Movie => &self.movie_discovery,
            MediaType::Tv => &self.tv_discovery,
            _ => unreachable!("discovery only supports movies and TV"),
        }
    }

    fn discovery_state_mut(&mut self, media_type: MediaType) -> &mut DiscoveryPageState {
        match media_type {
            MediaType::Movie => &mut self.movie_discovery,
            MediaType::Tv => &mut self.tv_discovery,
            _ => unreachable!("discovery only supports movies and TV"),
        }
    }

    fn ensure_discovery_loaded(&mut self, media_type: MediaType, cx: &mut Context<Self>) {
        if self.discovery_state(media_type).status == DiscoveryStatus::Idle {
            self.load_discovery_page(media_type, 1, true, cx);
        }
    }

    fn select_discovery_genre(
        &mut self,
        media_type: MediaType,
        genre: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        if self.discovery_state(media_type).selected_genre == genre {
            return;
        }
        self.discovery_state_mut(media_type).selected_genre = genre;
        self.load_discovery_page(media_type, 1, true, cx);
    }

    fn select_discovery_year(
        &mut self,
        media_type: MediaType,
        year: Option<u32>,
        cx: &mut Context<Self>,
    ) {
        if self.discovery_state(media_type).selected_year == year {
            return;
        }
        self.discovery_state_mut(media_type).selected_year = year;
        self.load_discovery_page(media_type, 1, true, cx);
    }

    fn select_discovery_sort(
        &mut self,
        media_type: MediaType,
        sort: DiscoverSort,
        cx: &mut Context<Self>,
    ) {
        if self.discovery_state(media_type).sort == sort {
            return;
        }
        self.discovery_state_mut(media_type).sort = sort;
        self.load_discovery_page(media_type, 1, true, cx);
    }

    fn load_more_discovery(&mut self, media_type: MediaType, cx: &mut Context<Self>) {
        let state = self.discovery_state(media_type);
        if state.loading_more
            || state.status == DiscoveryStatus::Loading
            || state.page >= state.total_pages
        {
            return;
        }
        let next_page = state.page.saturating_add(1).min(500);
        self.load_discovery_page(media_type, next_page, false, cx);
    }

    fn load_discovery_page(
        &mut self,
        media_type: MediaType,
        page: u32,
        reset: bool,
        cx: &mut Context<Self>,
    ) {
        if !matches!(media_type, MediaType::Movie | MediaType::Tv) || !(1..=500).contains(&page) {
            return;
        }
        let state = self.discovery_state_mut(media_type);
        state.cancellation.cancel();
        state.generation = state.generation.wrapping_add(1);
        let generation = state.generation;
        let cancellation = CancellationToken::new();
        state.cancellation = cancellation.clone();
        let genre = state.selected_genre;
        let year = state.selected_year;
        let sort = state.sort;
        let existing = if reset {
            HashSet::new()
        } else {
            state
                .cards
                .iter()
                .map(|card| crate::discovery::media_identity(card.media_type, card.id))
                .collect::<HashSet<_>>()
        };
        if reset {
            state.cards.clear();
            state.page = 0;
            state.total_pages = 1;
            state.status = DiscoveryStatus::Loading;
            state.error = None;
            state.loading_more = false;
        } else {
            state.loading_more = true;
            state.error = None;
        }
        let metadata = self.metadata.clone();
        let artwork = self.artwork.clone();
        let runtime = self.tokio.clone();
        let task_token = cancellation.clone();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let genre_token = task_token.child_token();
            let discover_token = task_token.child_token();
            let (genre_result, response_result) = tokio::join!(
                metadata.genres(media_type, "en-US", &genre_token),
                metadata.discover(
                    DiscoverRequest {
                        media_type,
                        genre_id: genre,
                        year,
                        sort,
                        page,
                        language: "en-US",
                    },
                    &discover_token
                ),
            );
            if task_token.is_cancelled() {
                return;
            }
            let response = match response_result {
                Ok(response) => response,
                Err(error) => {
                    let message = metadata_error_message(&error);
                    let _ = this.update(cx, |this, cx| {
                        let state = this.discovery_state_mut(media_type);
                        if state.generation != generation || task_token.is_cancelled() {
                            return;
                        }
                        state.loading_more = false;
                        state.error = Some(message);
                        if state.cards.is_empty() {
                            state.status = DiscoveryStatus::Failed;
                        }
                        cx.notify();
                    });
                    return;
                }
            };
            let genres = genre_result
                .ok()
                .map(|response| response.genres)
                .unwrap_or_default();
            let total_pages = response.total_pages.unwrap_or(page).clamp(1, 500);
            let mut cards = Vec::new();
            let mut image_requests = Vec::new();
            let mut seen = existing;
            for media in response.results.into_iter().take(24) {
                let kind = media.media_type.unwrap_or(media_type);
                if !matches!(kind, MediaType::Movie | MediaType::Tv)
                    || !seen.insert(crate::discovery::media_identity(kind, media.id))
                {
                    continue;
                }
                if let Some(path) = media.poster_path.clone() {
                    image_requests.push((cards.len(), path));
                }
                let label = if kind == MediaType::Movie {
                    "MOVIE"
                } else {
                    "TV SERIES"
                };
                let detail = media
                    .release_year()
                    .map(|year| format!("{label} · {year}"))
                    .unwrap_or_else(|| label.to_owned());
                cards.push(SearchMediaCard {
                    id: media.id,
                    media_type: kind,
                    title: media.display_title().to_owned(),
                    detail,
                    overview: media.overview,
                    vote_average: media.vote_average,
                    poster: None,
                });
            }
            let images = if image_requests.is_empty() {
                Vec::new()
            } else {
                let image_token = task_token.clone();
                runtime
                    .spawn(async move {
                        let mut jobs = JoinSet::new();
                        for (index, path) in image_requests {
                            let service = artwork.clone();
                            let token = image_token.child_token();
                            jobs.spawn(async move {
                                (
                                    index,
                                    load_gpui_artwork(service, path, ArtworkSize::W342, token)
                                        .await,
                                )
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
                    .await
                    .unwrap_or_default()
            };
            if task_token.is_cancelled() {
                return;
            }
            for (index, image) in images {
                if let Some(card) = cards.get_mut(index) {
                    card.poster = Some(image);
                }
            }
            let _ = this.update(cx, |this, cx| {
                let state = this.discovery_state_mut(media_type);
                if state.generation != generation || task_token.is_cancelled() {
                    return;
                }
                if !genres.is_empty() {
                    state.genres = genres;
                }
                if reset {
                    state.cards = cards;
                } else {
                    let mut identities = state
                        .cards
                        .iter()
                        .map(|card| crate::discovery::media_identity(card.media_type, card.id))
                        .collect::<HashSet<_>>();
                    for card in cards {
                        if identities
                            .insert(crate::discovery::media_identity(card.media_type, card.id))
                        {
                            state.cards.push(card);
                        }
                    }
                }
                state.page = page;
                state.total_pages = total_pages;
                state.loading_more = false;
                state.error = None;
                state.status = if state.cards.is_empty() {
                    DiscoveryStatus::Empty
                } else {
                    DiscoveryStatus::Results
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn open_discovery_result(
        &mut self,
        media_type: MediaType,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(card) = self.discovery_state(media_type).cards.get(index) else {
            return;
        };
        let (id, kind, title) = (card.id, card.media_type, card.title.clone());
        if matches!(kind, MediaType::Movie | MediaType::Tv) {
            self.load_details(id, kind, title, cx);
        }
    }

    fn discovery_sort_button(
        &self,
        media_type: MediaType,
        sort: DiscoverSort,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = self.discovery_state(media_type).sort == sort;
        let label = match sort {
            DiscoverSort::Popularity => "Most popular",
            DiscoverSort::Rating => "Top rated",
            DiscoverSort::Newest => "Newest",
            DiscoverSort::Revenue => "Highest grossing",
        };
        div()
            .id(format!(
                "discover-sort-{}-{label}",
                if media_type == MediaType::Movie {
                    "movie"
                } else {
                    "tv"
                }
            ))
            .tab_index(17)
            .rounded_md()
            .border_1()
            .border_color(rgb(if active { Theme::ACCENT } else { Theme::BORDER }))
            .bg(rgb(if active {
                Theme::ACCENT_DARK
            } else {
                Theme::SURFACE
            }))
            .px_3()
            .py_2()
            .cursor_pointer()
            .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
            .on_click(
                cx.listener(move |this, _, _, cx| this.select_discovery_sort(media_type, sort, cx)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(if active {
                        Theme::ACCENT
                    } else {
                        Theme::TEXT_SECONDARY
                    }))
                    .child(label),
            )
    }

    fn render_discovery(&self, media_type: MediaType, cx: &mut Context<Self>) -> gpui::Div {
        let state = self.discovery_state(media_type);
        let movie = media_type == MediaType::Movie;
        let title = if movie { "Movies" } else { "TV Shows" };
        let mut page = div().w_full().flex().flex_col().gap_5().child(page_intro(
            "DISCOVER",
            title,
            if movie {
                "Explore films by genre and release trends, powered by SENZO's hosted catalogue."
            } else {
                "Explore series by genre and first-air trends, powered by SENZO's hosted catalogue."
            },
        ));
        let mut sorts = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(self.discovery_sort_button(media_type, DiscoverSort::Popularity, cx))
            .child(self.discovery_sort_button(media_type, DiscoverSort::Rating, cx))
            .child(self.discovery_sort_button(media_type, DiscoverSort::Newest, cx));
        if movie {
            sorts = sorts.child(self.discovery_sort_button(media_type, DiscoverSort::Revenue, cx));
        }
        page = page.child(sorts);

        let all_selected = state.selected_genre.is_none();
        let mut genres = div().flex().flex_wrap().items_center().gap_2().child(
            div()
                .id("discover-genre-all")
                .tab_index(18)
                .rounded_md()
                .border_1()
                .border_color(rgb(if all_selected {
                    Theme::ACCENT
                } else {
                    Theme::BORDER
                }))
                .bg(rgb(if all_selected {
                    Theme::ACCENT_DARK
                } else {
                    Theme::SURFACE
                }))
                .px_3()
                .py_2()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select_discovery_genre(media_type, None, cx)
                }))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(if all_selected {
                            Theme::ACCENT
                        } else {
                            Theme::TEXT_SECONDARY
                        }))
                        .child("All genres"),
                ),
        );
        for genre in state.genres.iter().take(14) {
            let id = genre.id;
            let selected = state.selected_genre == Some(id);
            let name = genre.name.clone();
            genres = genres.child(
                div()
                    .id(format!("discover-genre-{id}"))
                    .tab_index(19)
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(if selected {
                        Theme::ACCENT
                    } else {
                        Theme::BORDER
                    }))
                    .bg(rgb(if selected {
                        Theme::ACCENT_DARK
                    } else {
                        Theme::SURFACE
                    }))
                    .px_3()
                    .py_2()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_discovery_genre(media_type, Some(id), cx)
                    }))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(if selected {
                                Theme::ACCENT
                            } else {
                                Theme::TEXT_SECONDARY
                            }))
                            .child(name),
                    ),
            );
        }
        page = page.child(genres);

        let mut years = div().flex().flex_wrap().items_center().gap_2().child(
            div()
                .id("discover-year-any")
                .tab_index(20)
                .rounded_md()
                .border_1()
                .border_color(rgb(if state.selected_year.is_none() {
                    Theme::ACCENT
                } else {
                    Theme::BORDER
                }))
                .bg(rgb(if state.selected_year.is_none() {
                    Theme::ACCENT_DARK
                } else {
                    Theme::SURFACE
                }))
                .px_3()
                .py_2()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select_discovery_year(media_type, None, cx)
                }))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(if state.selected_year.is_none() {
                            Theme::ACCENT
                        } else {
                            Theme::TEXT_SECONDARY
                        }))
                        .child("Any year"),
                ),
        );
        for year in (2017..=2026).rev() {
            let selected = state.selected_year == Some(year);
            years = years.child(
                div()
                    .id(format!("discover-year-{year}"))
                    .tab_index(21)
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(if selected {
                        Theme::ACCENT
                    } else {
                        Theme::BORDER
                    }))
                    .bg(rgb(if selected {
                        Theme::ACCENT_DARK
                    } else {
                        Theme::SURFACE
                    }))
                    .px_3()
                    .py_2()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_discovery_year(media_type, Some(year), cx)
                    }))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(if selected {
                                Theme::ACCENT
                            } else {
                                Theme::TEXT_SECONDARY
                            }))
                            .child(year.to_string()),
                    ),
            );
        }
        page = page.child(years);

        if state.status == DiscoveryStatus::Loading && state.cards.is_empty() {
            page = page.child(status_panel(
                StatusTone::Loading,
                "LOADING CATALOGUE",
                "Finding titles",
                "Fetching genre options and matching titles from the hosted metadata service.",
            ));
            let mut skeletons = div().w_full().flex().flex_wrap().gap_4();
            for _ in 0..8 {
                skeletons = skeletons.child(poster_card(" ", " ", None, true));
            }
            return page.child(skeletons);
        }
        if state.status == DiscoveryStatus::Failed {
            page = page.child(status_panel(
                StatusTone::Error,
                "DISCOVERY UNAVAILABLE",
                "Could not load this catalogue",
                state
                    .error
                    .as_deref()
                    .unwrap_or("Check your connection and retry."),
            ));
            page = page.child(
                div()
                    .id("retry-discovery")
                    .tab_index(40)
                    .rounded_md()
                    .bg(rgb(Theme::ACCENT_DARK))
                    .px_3()
                    .py_2()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.load_discovery_page(media_type, 1, true, cx)
                    }))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::ACCENT))
                            .child("Retry"),
                    ),
            );
        }
        if state.cards.is_empty() && state.status == DiscoveryStatus::Empty {
            page = page.child(status_panel(
                StatusTone::Empty,
                "NO MATCHING TITLES",
                "Nothing matched these filters",
                "Choose another genre or sort order to broaden the catalogue.",
            ));
        }
        if !state.cards.is_empty() {
            let mut cards = div().w_full().flex().flex_wrap().gap_4();
            for (index, card) in state.cards.iter().enumerate() {
                cards = cards.child(
                    div()
                        .id(format!("discover-card-{}-{index}", card.id))
                        .tab_index(50 + index as isize)
                        .rounded_lg()
                        .border_1()
                        .border_color(rgb(Theme::BORDER))
                        .p_1()
                        .cursor_pointer()
                        .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                        .aria_label(format!("Open {}", card.title))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_discovery_result(media_type, index, cx)
                        }))
                        .child(poster_card(
                            &card.title,
                            &card.detail,
                            card.poster.clone(),
                            false,
                        )),
                );
            }
            page = page.child(cards);
        }
        if state.loading_more {
            page = page.child(status_panel(
                StatusTone::Loading,
                "LOADING MORE",
                "Fetching the next page",
                "Your current catalogue and filters are preserved.",
            ));
        } else if state.page < state.total_pages && state.status == DiscoveryStatus::Results {
            page = page.child(
                div()
                    .id("discover-load-more")
                    .tab_index(90)
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(Theme::BORDER_STRONG))
                    .bg(rgb(Theme::SURFACE_RAISED))
                    .px_4()
                    .py_3()
                    .cursor_pointer()
                    .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.load_more_discovery(media_type, cx)),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::ACCENT))
                            .child("Load more titles"),
                    ),
            );
        }
        if let Some(error) = state.error.as_deref().filter(|_| !state.cards.is_empty()) {
            page = page.child(status_panel(
                StatusTone::Error,
                "MORE TITLES UNAVAILABLE",
                "Could not load the next page",
                error,
            ));
        }
        page
    }

    fn update_search_query(&mut self, query: String, cx: &mut Context<Self>) {
        if self.search_query == query {
            return;
        }
        self.search_query = query;
        self.search_page = 1;
        self.search_total_pages = 1;
        self.schedule_search(cx);
    }

    fn set_search_filter(&mut self, filter: SearchFilter, cx: &mut Context<Self>) {
        if self.search_filter == filter {
            return;
        }
        self.search_filter = filter;
        self.search_page = 1;
        self.search_total_pages = 1;
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
        let search_page = self.search_page;
        let history_query = query.clone();
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
                        .search(&query, search_type, search_page, "en-US", &request_token)
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
            let response_total_pages = response.total_pages.unwrap_or(search_page).clamp(1, 500);
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
                this.search_total_pages = response_total_pages;
                this.record_search_query(&history_query);
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

    fn load_plugin_registry(&mut self, cx: &mut Context<Self>) {
        if self.smoke_test || self.plugin_busy || self.plugin_registry_loaded {
            return;
        }
        self.plugin_busy = true;
        self.plugin_error = None;
        let manager = self.plugin_manager.clone();
        let runtime = self.tokio.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn_blocking(move || manager.list_state("default"))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.plugin_busy = false;
                match result {
                    Ok(Ok(state)) => {
                        this.plugin_state = state;
                        this.plugin_registry_loaded = true;
                    }
                    Ok(Err(error)) => this.plugin_error = Some(error.to_string()),
                    Err(error) => {
                        this.plugin_error = Some(format!("Plugin registry task failed: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn add_plugin_repository(&mut self, cx: &mut Context<Self>) {
        if self.plugin_busy {
            return;
        }
        let raw_url = self.plugin_repository_input.read(cx).value().to_string();
        if raw_url.trim().is_empty() {
            self.plugin_error = Some("Enter a plugin repository URL first.".to_owned());
            cx.notify();
            return;
        }
        let manager = self.plugin_manager.clone();
        let runtime = self.tokio.clone();
        self.plugin_busy = true;
        self.plugin_error = None;
        self.plugin_notice = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime.spawn_blocking(move || manager.add_repository("default", &raw_url)).await;
            let _ = this.update(cx, |this, cx| {
                this.plugin_busy = false;
                match result {
                    Ok(Ok(state)) => {
                        this.plugin_state = state;
                        this.plugin_registry_loaded = true;
                        this.plugin_notice = Some("Plugin repository installed. Review each scraper and grant network access explicitly before testing or running it.".to_owned());
                    }
                    Ok(Err(error)) => this.plugin_error = Some(error.to_string()),
                    Err(error) => this.plugin_error = Some(format!("Plugin repository install failed: {error}")),
                }
                cx.notify();
            });
        }).detach();
    }

    fn refresh_plugin_repository(&mut self, manifest_url: String, cx: &mut Context<Self>) {
        if self.plugin_busy {
            return;
        }
        let manager = self.plugin_manager.clone();
        let runtime = self.tokio.clone();
        self.plugin_busy = true;
        self.plugin_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime.spawn_blocking(move || manager.refresh_repository("default", &manifest_url)).await;
            let _ = this.update(cx, |this, cx| {
                this.plugin_busy = false;
                match result {
                    Ok(Ok(state)) => {
                        this.plugin_state = state;
                        this.plugin_notice = Some("Plugin repository refreshed; existing enable and permission choices were preserved.".to_owned());
                    }
                    Ok(Err(error)) => this.plugin_error = Some(error.to_string()),
                    Err(error) => this.plugin_error = Some(format!("Plugin refresh failed: {error}")),
                }
                cx.notify();
            });
        }).detach();
    }

    fn remove_plugin_repository(&mut self, manifest_url: String, cx: &mut Context<Self>) {
        if self.plugin_busy {
            return;
        }
        let manager = self.plugin_manager.clone();
        let runtime = self.tokio.clone();
        self.plugin_busy = true;
        self.plugin_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn_blocking(move || manager.remove_repository("default", &manifest_url))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.plugin_busy = false;
                match result {
                    Ok(Ok(state)) => {
                        this.plugin_state = state;
                        this.plugin_stream_results.clear();
                        this.plugin_selected_scraper_id = None;
                        this.plugin_settings_layout = None;
                        this.plugin_settings_preview = None;
                        this.plugin_notice = Some(
                            "Plugin repository and cached scraper scripts removed.".to_owned(),
                        );
                    }
                    Ok(Err(error)) => this.plugin_error = Some(error.to_string()),
                    Err(error) => {
                        this.plugin_error = Some(format!("Plugin removal failed: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_plugin_enabled(&mut self, scraper_id: String, enabled: bool, cx: &mut Context<Self>) {
        if self.plugin_busy {
            return;
        }
        let manager = self.plugin_manager.clone();
        let runtime = self.tokio.clone();
        self.plugin_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn_blocking(move || manager.set_enabled("default", &scraper_id, enabled))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.plugin_busy = false;
                match result {
                    Ok(Ok(state)) => this.plugin_state = state,
                    Ok(Err(error)) => this.plugin_error = Some(error.to_string()),
                    Err(error) => {
                        this.plugin_error = Some(format!("Plugin enable state failed: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_plugin_network_permission(
        &mut self,
        scraper_id: String,
        granted: bool,
        cx: &mut Context<Self>,
    ) {
        if self.plugin_busy {
            return;
        }
        let manager = self.plugin_manager.clone();
        let runtime = self.tokio.clone();
        self.plugin_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn_blocking(move || {
                    manager.set_network_permission("default", &scraper_id, granted)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.plugin_busy = false;
                match result {
                    Ok(Ok(state)) => {
                        this.plugin_state = state;
                        this.plugin_notice = Some(if granted {
                            "Network access granted for this scraper only.".to_owned()
                        } else {
                            "Network access revoked for this scraper.".to_owned()
                        });
                    }
                    Ok(Err(error)) => this.plugin_error = Some(error.to_string()),
                    Err(error) => {
                        this.plugin_error =
                            Some(format!("Plugin permission update failed: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_plugins_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.plugin_busy {
            return;
        }
        let manager = self.plugin_manager.clone();
        let runtime = self.tokio.clone();
        self.plugin_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn_blocking(move || manager.set_plugins_enabled("default", enabled))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.plugin_busy = false;
                match result {
                    Ok(Ok(state)) => {
                        this.plugin_state = state;
                        this.plugin_notice = Some(if enabled {
                            "Plugin execution enabled for the Default profile.".to_owned()
                        } else {
                            "All plugin execution disabled for the Default profile.".to_owned()
                        });
                    }
                    Ok(Err(error)) => this.plugin_error = Some(error.to_string()),
                    Err(error) => {
                        this.plugin_error = Some(format!("Plugin setting update failed: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn test_plugin_scraper(&mut self, scraper_id: String, cx: &mut Context<Self>) {
        if self.plugin_busy {
            return;
        }
        let manager = self.plugin_manager.clone();
        let runtime = self.tokio.clone();
        self.plugin_busy = true;
        self.plugin_error = None;
        self.plugin_stream_results.clear();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn(async move { manager.test_scraper("default", &scraper_id).await })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.plugin_busy = false;
                match result {
                    Ok(Ok(streams)) => {
                        this.plugin_stream_results = streams;
                        this.plugin_notice = Some(format!(
                            "Scraper compatibility test finished: {} stream(s) returned.",
                            this.plugin_stream_results.len()
                        ));
                    }
                    Ok(Err(error)) => this.plugin_error = Some(error.to_string()),
                    Err(error) => {
                        this.plugin_error = Some(format!("Scraper test worker failed: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn execute_plugin_scraper(&mut self, scraper_id: String, cx: &mut Context<Self>) {
        if self.plugin_busy {
            return;
        }
        let input = ScraperInput {
            tmdb_id: self.plugin_tmdb_id_input.read(cx).value().trim().to_owned(),
            media_type: self
                .plugin_media_type_input
                .read(cx)
                .value()
                .trim()
                .to_owned(),
            season: self
                .plugin_season_input
                .read(cx)
                .value()
                .trim()
                .parse()
                .ok(),
            episode: self
                .plugin_episode_input
                .read(cx)
                .value()
                .trim()
                .parse()
                .ok(),
        };
        let manager = self.plugin_manager.clone();
        let runtime = self.tokio.clone();
        self.plugin_busy = true;
        self.plugin_error = None;
        self.plugin_stream_results.clear();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn(async move { manager.execute_scraper("default", &scraper_id, input).await })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.plugin_busy = false;
                match result {
                    Ok(Ok(streams)) => {
                        this.plugin_stream_results = streams;
                        this.plugin_notice = Some(format!(
                            "Scraper execution completed: {} stream(s) returned.",
                            this.plugin_stream_results.len()
                        ));
                    }
                    Ok(Err(error)) => this.plugin_error = Some(error.to_string()),
                    Err(error) => {
                        this.plugin_error =
                            Some(format!("Scraper execution worker failed: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_plugin_settings(&mut self, scraper_id: String, cx: &mut Context<Self>) {
        if self.plugin_busy {
            return;
        }
        let manager = self.plugin_manager.clone();
        let settings_manager = manager.clone();
        let settings_scraper_id = scraper_id.clone();
        let runtime = self.tokio.clone();
        self.plugin_busy = true;
        self.plugin_error = None;
        self.plugin_selected_scraper_id = Some(scraper_id.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let saved = runtime.spawn_blocking(move || {
                settings_manager.load_scraper_settings("default", &settings_scraper_id)
            }).await;
            let layout = manager.scraper_settings_layout("default", &scraper_id).await;
            let _ = this.update(cx, |this, cx| {
                this.plugin_busy = false;
                match saved {
                    Ok(Ok(value)) => this.plugin_settings_preview = Some(value),
                    Ok(Err(error)) => this.plugin_error = Some(error.to_string()),
                    Err(error) => this.plugin_error = Some(format!("Plugin settings read failed: {error}")),
                }
                match layout {
                    Ok(value) => this.plugin_settings_layout = Some(value),
                    Err(error) => this.plugin_error = Some(error.to_string()),
                }
                this.plugin_notice = Some("Settings schema and saved JSON loaded below. Paste edited JSON into the settings field, then save.".to_owned());
                cx.notify();
            });
        }).detach();
    }

    fn save_plugin_settings(&mut self, cx: &mut Context<Self>) {
        if self.plugin_busy {
            return;
        }
        let Some(scraper_id) = self.plugin_selected_scraper_id.clone() else {
            self.plugin_error = Some("Load a scraper's settings before saving.".to_owned());
            cx.notify();
            return;
        };
        let raw_json = self.plugin_settings_input.read(cx).value().to_string();
        let manager = self.plugin_manager.clone();
        let runtime = self.tokio.clone();
        let settings_json = raw_json.clone();
        self.plugin_busy = true;
        self.plugin_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn_blocking(move || {
                    manager.save_scraper_settings("default", &scraper_id, &settings_json)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.plugin_busy = false;
                match result {
                    Ok(Ok(())) => {
                        this.plugin_settings_preview = Some(raw_json);
                        this.plugin_notice = Some(
                            "Plugin settings stored in the operating-system credential store."
                                .to_owned(),
                        );
                    }
                    Ok(Err(error)) => this.plugin_error = Some(error.to_string()),
                    Err(error) => {
                        this.plugin_error = Some(format!("Plugin settings save failed: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_plugin_settings(&self, cx: &mut Context<Self>) -> gpui::Div {
        let repository_input = Input::new(&self.plugin_repository_input)
            .tab_index(63)
            .aria_label("Nuvio plugin repository URL")
            .cleanable(true)
            .w_full()
            .bg(rgb(Theme::SURFACE))
            .border_1()
            .border_color(rgb(Theme::BORDER))
            .rounded_lg()
            .px_4()
            .py_3();
        let settings_input = Input::new(&self.plugin_settings_input)
            .tab_index(69)
            .aria_label("Plugin settings JSON")
            .cleanable(true)
            .w_full()
            .bg(rgb(Theme::SURFACE))
            .border_1()
            .border_color(rgb(Theme::BORDER))
            .rounded_lg()
            .px_4()
            .py_3();
        let tmdb_id_input = Input::new(&self.plugin_tmdb_id_input)
            .tab_index(65)
            .aria_label("TMDB identifier for plugin execution")
            .w_full()
            .bg(rgb(Theme::SURFACE))
            .border_1()
            .border_color(rgb(Theme::BORDER))
            .rounded_lg()
            .px_3()
            .py_2();
        let media_type_input = Input::new(&self.plugin_media_type_input)
            .tab_index(66)
            .aria_label("Media type for plugin execution")
            .w_full()
            .bg(rgb(Theme::SURFACE))
            .border_1()
            .border_color(rgb(Theme::BORDER))
            .rounded_lg()
            .px_3()
            .py_2();
        let season_input = Input::new(&self.plugin_season_input)
            .tab_index(67)
            .aria_label("Season number for plugin execution")
            .w_full()
            .bg(rgb(Theme::SURFACE))
            .border_1()
            .border_color(rgb(Theme::BORDER))
            .rounded_lg()
            .px_3()
            .py_2();
        let episode_input = Input::new(&self.plugin_episode_input)
            .tab_index(68)
            .aria_label("Episode number for plugin execution")
            .w_full()
            .bg(rgb(Theme::SURFACE))
            .border_1()
            .border_color(rgb(Theme::BORDER))
            .rounded_lg()
            .px_3()
            .py_2();

        let mut section = div()
            .w_full()
            .flex()
            .flex_col()
            .gap_3()
            .rounded_lg()
            .border_1()
            .border_color(rgb(Theme::BORDER))
            .bg(rgb(Theme::SURFACE))
            .p_4()
            .child(div().text_sm().text_color(rgb(Theme::ACCENT)).child("PLUGIN RUNTIME"))
            .child(div().text_lg().text_color(rgb(Theme::TEXT_PRIMARY)).child("Nuvio scrapers"))
            .child(div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child("JavaScript scrapers run in isolated QuickJS sandboxes. This system is independent from Stremio add-ons. Repositories are opt-in; each scraper needs its own network permission."))
            .child(repository_input)
            .child(self.addon_control(
                "plugin-add-repository",
                if self.plugin_busy { "Working…" } else { "Install plugin repository" },
                64,
                cx.listener(|this, _, _, cx| this.add_plugin_repository(cx)),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().flex_1().text_xs().text_color(rgb(Theme::TEXT_SECONDARY))
                        .child(if self.plugin_state.plugins_enabled { "Plugin execution is enabled for the Default profile." } else { "All plugin execution is disabled for the Default profile." }))
                    .child(self.addon_control(
                        "plugin-global-enable",
                        if self.plugin_state.plugins_enabled { "Disable all" } else { "Enable all" },
                        70,
                        cx.listener(|this, _, _, cx| {
                            let enabled = !this.plugin_state.plugins_enabled;
                            this.set_plugins_enabled(enabled, cx);
                        }),
                    )),
            );

        if self.plugin_state.repositories.is_empty() {
            section = section.child(status_panel(
                StatusTone::Empty,
                "NO PLUGIN REPOSITORIES",
                "No scraper repositories installed",
                "Add a Nuvio-compatible manifest URL above. Installation downloads the manifest and declared JavaScript files but does not grant scraper network access.",
            ));
        } else {
            section = section.child(div().text_sm().text_color(rgb(Theme::TEXT_PRIMARY)).child(
                format!("Repositories · {}", self.plugin_state.repositories.len()),
            ));
            for (index, repository) in self.plugin_state.repositories.iter().enumerate() {
                let refresh_url = repository.manifest_url.clone();
                let remove_url = repository.manifest_url.clone();
                section = section.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(Theme::BORDER))
                        .p_3()
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
                                        .child(repository.name.clone()),
                                )
                                .child(div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child(
                                    format!(
                                        "v{} · {} scraper(s)",
                                        repository.version, repository.scraper_count
                                    ),
                                )),
                        )
                        .child(self.addon_control(
                            format!("plugin-refresh-repository-{index}"),
                            "Refresh",
                            71 + index as isize * 3,
                            cx.listener(move |this, _, _, cx| {
                                this.refresh_plugin_repository(refresh_url.clone(), cx)
                            }),
                        ))
                        .child(self.addon_control(
                            format!("plugin-remove-repository-{index}"),
                            "Remove",
                            72 + index as isize * 3,
                            cx.listener(move |this, _, _, cx| {
                                this.remove_plugin_repository(remove_url.clone(), cx)
                            }),
                        )),
                );
            }
        }

        if self.plugin_state.scrapers.is_empty() {
            section = section.child(status_panel(
                StatusTone::Empty,
                "NO SCRAPERS",
                "Repository has no scraper definitions",
                "Scrapers declared by installed manifests appear here.",
            ));
        } else {
            section = section.child(
                div()
                    .text_sm()
                    .text_color(rgb(Theme::TEXT_PRIMARY))
                    .child(format!("Scrapers · {}", self.plugin_state.scrapers.len())),
            );
            for (index, scraper) in self.plugin_state.scrapers.iter().enumerate() {
                let enable_id = scraper.id.clone();
                let permission_id = scraper.id.clone();
                let test_id = scraper.id.clone();
                let execute_id = scraper.id.clone();
                let settings_id = scraper.id.clone();
                let scraper_enabled = scraper.enabled;
                let network_granted = scraper.network_access_granted;
                let name = scraper.name.clone();
                section =
                    section.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(Theme::BORDER))
                            .p_3()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(Theme::TEXT_PRIMARY))
                                    .child(name),
                            )
                            .child(div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child(
                                format!(
                                    "{} · v{} · {}",
                                    scraper.id,
                                    scraper.version,
                                    scraper.supported_types.join(", ")
                                ),
                            ))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(if scraper.network_access_granted {
                                        Theme::SUCCESS
                                    } else {
                                        Theme::WARNING
                                    }))
                                    .child(if scraper.network_access_granted {
                                        "Network permission granted"
                                    } else {
                                        "Network permission not granted"
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(self.addon_control(
                                        format!("plugin-enabled-{index}"),
                                        if scraper.enabled { "Disable" } else { "Enable" },
                                        100 + index as isize * 5,
                                        cx.listener(move |this, _, _, cx| {
                                            this.set_plugin_enabled(
                                                enable_id.clone(),
                                                !scraper_enabled,
                                                cx,
                                            )
                                        }),
                                    ))
                                    .child(self.addon_control(
                                        format!("plugin-network-{index}"),
                                        if scraper.network_access_granted {
                                            "Revoke network"
                                        } else {
                                            "Grant network"
                                        },
                                        101 + index as isize * 5,
                                        cx.listener(move |this, _, _, cx| {
                                            this.set_plugin_network_permission(
                                                permission_id.clone(),
                                                !network_granted,
                                                cx,
                                            )
                                        }),
                                    ))
                                    .child(self.addon_control(
                                        format!("plugin-test-{index}"),
                                        "Test scraper",
                                        102 + index as isize * 5,
                                        cx.listener(move |this, _, _, cx| {
                                            this.test_plugin_scraper(test_id.clone(), cx)
                                        }),
                                    ))
                                    .child(self.addon_control(
                                        format!("plugin-execute-{index}"),
                                        "Execute",
                                        103 + index as isize * 5,
                                        cx.listener(move |this, _, _, cx| {
                                            this.execute_plugin_scraper(execute_id.clone(), cx)
                                        }),
                                    ))
                                    .child(self.addon_control(
                                        format!("plugin-settings-{index}"),
                                        "Settings",
                                        104 + index as isize * 5,
                                        cx.listener(move |this, _, _, cx| {
                                            this.load_plugin_settings(settings_id.clone(), cx)
                                        }),
                                    )),
                            ),
                    );
            }
        }

        section = section.child(
            div()
                .text_sm()
                .text_color(rgb(Theme::TEXT_PRIMARY))
                .child("Execute a scraper"),
        );
        section = section.child(
            div()
                .flex()
                .gap_2()
                .child(div().flex_1().child(tmdb_id_input))
                .child(div().flex_1().child(media_type_input)),
        );
        section = section.child(
            div()
                .flex()
                .gap_2()
                .child(div().flex_1().child(season_input))
                .child(div().flex_1().child(episode_input)),
        );
        section = section.child(settings_input);
        section = section.child(self.addon_control(
            "plugin-save-settings",
            "Save selected scraper settings JSON",
            110,
            cx.listener(|this, _, _, cx| this.save_plugin_settings(cx)),
        ));

        if let Some(selected) = self.plugin_selected_scraper_id.as_deref() {
            section = section.child(
                div()
                    .text_xs()
                    .text_color(rgb(Theme::TEXT_MUTED))
                    .child(format!("Selected settings: {selected}")),
            );
        }
        if let Some(schema) = self.plugin_settings_layout.as_deref() {
            section = section.child(
                div()
                    .text_xs()
                    .text_color(rgb(Theme::TEXT_SECONDARY))
                    .child(format!(
                        "onSettings schema: {}",
                        schema.chars().take(6000).collect::<String>()
                    )),
            );
        }
        if let Some(saved) = self.plugin_settings_preview.as_deref() {
            section = section.child(
                div()
                    .text_xs()
                    .text_color(rgb(Theme::TEXT_SECONDARY))
                    .child(format!(
                        "Saved settings JSON: {}",
                        saved.chars().take(6000).collect::<String>()
                    )),
            );
        }
        if let Some(error) = self.plugin_error.as_deref() {
            section = section.child(status_panel(
                StatusTone::Error,
                "PLUGIN OPERATION FAILED",
                "Plugin runtime",
                error,
            ));
        }
        if let Some(notice) = self.plugin_notice.as_deref() {
            section = section.child(status_panel(
                StatusTone::Info,
                "PLUGIN STATUS",
                "Scraper update",
                notice,
            ));
        }
        if !self.plugin_stream_results.is_empty() {
            section = section.child(div().text_sm().text_color(rgb(Theme::TEXT_PRIMARY)).child(
                format!("Stream results · {}", self.plugin_stream_results.len()),
            ));
            for (index, stream) in self.plugin_stream_results.iter().enumerate() {
                let target = match &stream.target {
                    senzo_plugins::StreamTarget::Http(url) => format!("HTTP · {url}"),
                    senzo_plugins::StreamTarget::Torrent {
                        info_hash,
                        file_index,
                    } => format!("Torrent · {info_hash} · file {file_index:?}"),
                    senzo_plugins::StreamTarget::External(url) => format!("External · {url}"),
                    senzo_plugins::StreamTarget::Youtube(id) => format!("YouTube · {id}"),
                };
                section =
                    section.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(Theme::BORDER))
                            .p_3()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(Theme::TEXT_PRIMARY))
                                    .child(format!("{} · {}", index + 1, stream.title)),
                            )
                            .child(div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child(
                                format!(
                                    "{} · {}",
                                    stream.provider,
                                    stream.quality.as_deref().unwrap_or("Quality unknown")
                                ),
                            ))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(Theme::TEXT_SECONDARY))
                                    .child(target),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(Theme::TEXT_MUTED))
                                    .child(format!("{} subtitle(s)", stream.subtitles.len())),
                            ),
                    );
            }
        }
        section
    }

    fn load_addon_registry(&mut self, cx: &mut Context<Self>) {
        // Navigation and native-window smoke tests must not schedule background
        // storage work on GPUI's deterministic single-threaded test scheduler.
        if self.smoke_test || self.addon_busy || self.addon_registry_loaded {
            return;
        }
        self.addon_busy = true;
        self.addon_error = None;
        let manager = self.addon_manager.clone();
        let runtime = self.tokio.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn_blocking(move || manager.list_addons("default"))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.addon_busy = false;
                match result {
                    Ok(Ok(addons)) => {
                        this.addon_state = addons;
                        this.addon_registry_loaded = true;
                    }
                    Ok(Err(error)) => this.addon_error = Some(error.to_string()),
                    Err(error) => {
                        this.addon_error = Some(format!("Add-on registry task failed: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn preview_addon(&mut self, cx: &mut Context<Self>) {
        if self.addon_busy {
            return;
        }
        let raw_url = self.addon_url_input.read(cx).value().to_string();
        if raw_url.trim().is_empty() {
            self.addon_error = Some("Enter a Stremio add-on manifest URL first.".to_owned());
            cx.notify();
            return;
        }
        let manager = self.addon_manager.clone();
        let runtime = self.tokio.clone();
        let allow_http = self.addon_allow_http;
        self.addon_busy = true;
        self.addon_error = None;
        self.addon_notice = None;
        self.addon_preview = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn(async move { manager.preview_addon(&raw_url, allow_http).await })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.addon_busy = false;
                match result {
                    Ok(Ok(preview)) => {
                        this.addon_preview = Some(preview);
                        this.addon_notice = Some(
                            "Manifest validated. Review the provider and confirm installation."
                                .to_owned(),
                        );
                    }
                    Ok(Err(error)) => this.addon_error = Some(error.to_string()),
                    Err(error) => {
                        this.addon_error = Some(format!("Manifest preview task failed: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn install_addon_preview(&mut self, cx: &mut Context<Self>) {
        if self.addon_busy {
            return;
        }
        let Some(preview) = self.addon_preview.clone() else {
            self.addon_error =
                Some("Preview a manifest before confirming installation.".to_owned());
            cx.notify();
            return;
        };
        let manager = self.addon_manager.clone();
        let runtime = self.tokio.clone();
        self.addon_busy = true;
        self.addon_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn_blocking(move || {
                    let installed = manager.install_preview("default", &preview);
                    let current = manager.list_addons("default");
                    (installed, current)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.addon_busy = false;
                match result {
                    Ok((Ok(addon), Ok(addons))) => {
                        this.addon_state = addons;
                        this.addon_preview = None;
                        this.addon_notice = Some(format!(
                            "{} was installed for the Default profile.",
                            addon
                                .manifest
                                .as_ref()
                                .map(|manifest| manifest.name.as_str())
                                .unwrap_or("Add-on")
                        ));
                        this.addon_catalog_results.clear();
                    }
                    Ok((Err(error), _)) => this.addon_error = Some(error.to_string()),
                    Ok((Ok(_), Err(error))) => {
                        this.addon_error = Some(format!(
                            "Installed, but the list could not be refreshed: {error}"
                        ))
                    }
                    Err(error) => {
                        this.addon_error = Some(format!("Installation task failed: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn change_addon_enabled(&mut self, addon_id: String, enabled: bool, cx: &mut Context<Self>) {
        if self.addon_busy {
            return;
        }
        let manager = self.addon_manager.clone();
        let runtime = self.tokio.clone();
        self.addon_busy = true;
        self.addon_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn_blocking(move || {
                    manager.set_enabled("default", &addon_id, enabled)?;
                    manager.list_addons("default")
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.addon_busy = false;
                match result {
                    Ok(Ok(addons)) => {
                        this.addon_state = addons;
                        this.addon_notice = Some(if enabled {
                            "Add-on enabled.".to_owned()
                        } else {
                            "Add-on disabled.".to_owned()
                        });
                    }
                    Ok(Err(error)) => this.addon_error = Some(error.to_string()),
                    Err(error) => {
                        this.addon_error = Some(format!("Add-on update task failed: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn remove_addon(&mut self, addon_id: String, cx: &mut Context<Self>) {
        if self.addon_busy {
            return;
        }
        let manager = self.addon_manager.clone();
        let runtime = self.tokio.clone();
        self.addon_busy = true;
        self.addon_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn_blocking(move || {
                    manager.remove_addon("default", &addon_id)?;
                    manager.list_addons("default")
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.addon_busy = false;
                match result {
                    Ok(Ok(addons)) => {
                        this.addon_state = addons;
                        this.addon_catalog_results.clear();
                        this.addon_stream_results.clear();
                        this.addon_subtitle_results.clear();
                        this.addon_notice = Some("Add-on removed from this profile.".to_owned());
                    }
                    Ok(Err(error)) => this.addon_error = Some(error.to_string()),
                    Err(error) => {
                        this.addon_error = Some(format!("Add-on removal task failed: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_addon(&mut self, addon_id: String, cx: &mut Context<Self>) {
        if self.addon_busy {
            return;
        }
        let manager = self.addon_manager.clone();
        let runtime = self.tokio.clone();
        self.addon_busy = true;
        self.addon_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn(async move {
                    let refresh = manager.refresh_addon("default", &addon_id).await;
                    let current = manager.list_addons("default");
                    (refresh, current)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.addon_busy = false;
                match result {
                    Ok((Ok(_), Ok(addons))) => {
                        this.addon_state = addons;
                        this.addon_notice = Some("Add-on manifest refreshed.".to_owned());
                    }
                    Ok((Err(error), Ok(addons))) => {
                        this.addon_state = addons;
                        this.addon_error = Some(format!(
                            "Refresh failed; the last valid manifest was retained. {error}"
                        ));
                    }
                    Ok((_, Err(error))) => {
                        this.addon_error = Some(format!("Could not reload add-ons: {error}"))
                    }
                    Err(error) => {
                        this.addon_error = Some(format!("Add-on refresh task failed: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_all_addons(&mut self, cx: &mut Context<Self>) {
        if self.addon_busy {
            return;
        }
        let manager = self.addon_manager.clone();
        let runtime = self.tokio.clone();
        self.addon_busy = true;
        self.addon_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime.spawn(async move {
                let outcomes = manager.refresh_all("default").await;
                let current = manager.list_addons("default");
                (outcomes, current)
            }).await;
            let _ = this.update(cx, |this, cx| {
                this.addon_busy = false;
                match result {
                    Ok((outcomes, Ok(addons))) => {
                        let failures = outcomes.iter().filter(|(_, result)| result.is_err()).count();
                        this.addon_state = addons;
                        this.addon_notice = Some(format!("Refreshed {} add-on(s); {failures} refresh failure(s). Existing valid manifests remain available.", outcomes.len()));
                    }
                    Ok((_, Err(error))) => this.addon_error = Some(error.to_string()),
                    Err(error) => this.addon_error = Some(format!("Refresh-all task failed: {error}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn browse_addon_catalog(
        &mut self,
        addon_id: String,
        media_type: String,
        catalog_id: String,
        cx: &mut Context<Self>,
    ) {
        if self.addon_busy {
            return;
        }
        let extras_raw = self.addon_catalog_extras_input.read(cx).value().to_string();
        let extras = match parse_addon_extras(&extras_raw) {
            Ok(extras) => extras,
            Err(error) => {
                self.addon_error = Some(error);
                cx.notify();
                return;
            }
        };
        let manager = self.addon_manager.clone();
        let runtime = self.tokio.clone();
        self.addon_busy = true;
        self.addon_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let request_addon_id = addon_id.clone();
            let request_catalog_id = catalog_id.clone();
            let result = runtime
                .spawn(async move {
                    manager
                        .fetch_catalog(
                            "default",
                            &request_addon_id,
                            &media_type,
                            &request_catalog_id,
                            &extras,
                        )
                        .await
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.addon_busy = false;
                match result {
                    Ok(Ok(items)) => {
                        let count = items.len();
                        this.addon_catalog_results
                            .retain(|(id, catalog, _)| id != &addon_id || catalog != &catalog_id);
                        this.addon_catalog_results
                            .push((addon_id, catalog_id, items));
                        this.addon_notice = Some(format!("Catalogue loaded: {count} title(s)."));
                    }
                    Ok(Err(error)) => this.addon_error = Some(error.to_string()),
                    Err(error) => {
                        this.addon_error = Some(format!("Catalogue request failed: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn find_addon_metadata(&mut self, addon_id: String, cx: &mut Context<Self>) {
        if self.addon_busy {
            return;
        }
        let media_type = self.addon_media_type_input.read(cx).value().to_string();
        let media_id = self.addon_media_id_input.read(cx).value().to_string();
        if media_type.trim().is_empty() || media_id.trim().is_empty() {
            self.addon_error =
                Some("Enter a media type and ID before requesting metadata.".to_owned());
            cx.notify();
            return;
        }
        let manager = self.addon_manager.clone();
        let runtime = self.tokio.clone();
        self.addon_busy = true;
        self.addon_error = None;
        self.addon_meta_result = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn(async move {
                    manager
                        .fetch_meta("default", &addon_id, media_type.trim(), media_id.trim())
                        .await
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.addon_busy = false;
                match result {
                    Ok(Ok(meta)) => {
                        this.addon_meta_result = Some(meta);
                        this.addon_result_message = Some(
                            "Metadata response validated against the requested media type and ID."
                                .to_owned(),
                        );
                    }
                    Ok(Err(error)) => this.addon_error = Some(error.to_string()),
                    Err(error) => {
                        this.addon_error = Some(format!("Metadata request failed: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn move_addon(&mut self, addon_id: String, direction: isize, cx: &mut Context<Self>) {
        if self.addon_busy {
            return;
        }
        let mut ids = self
            .addon_state
            .iter()
            .map(|addon| addon.id.clone())
            .collect::<Vec<_>>();
        let Some(current) = ids.iter().position(|id| id == &addon_id) else {
            return;
        };
        let target = current as isize + direction;
        if target < 0 || target >= ids.len() as isize {
            return;
        }
        ids.swap(current, target as usize);
        let manager = self.addon_manager.clone();
        let runtime = self.tokio.clone();
        self.addon_busy = true;
        self.addon_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn_blocking(move || {
                    manager.reorder_addons("default", &ids)?;
                    manager.list_addons("default")
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.addon_busy = false;
                match result {
                    Ok(Ok(addons)) => {
                        this.addon_state = addons;
                        this.addon_notice = Some("Provider order saved.".to_owned());
                    }
                    Ok(Err(error)) => this.addon_error = Some(error.to_string()),
                    Err(error) => {
                        this.addon_error = Some(format!("Provider reorder failed: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn find_addon_streams(&mut self, addon_id: String, cx: &mut Context<Self>) {
        if self.addon_busy {
            return;
        }
        let media_type = self.addon_media_type_input.read(cx).value().to_string();
        let media_id = self.addon_media_id_input.read(cx).value().to_string();
        if media_type.trim().is_empty() || media_id.trim().is_empty() {
            self.addon_error =
                Some("Enter a media type and ID before requesting streams.".to_owned());
            cx.notify();
            return;
        }
        let manager = self.addon_manager.clone();
        let runtime = self.tokio.clone();
        self.addon_busy = true;
        self.addon_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime.spawn(async move {
                manager.fetch_streams("default", &addon_id, media_type.trim(), media_id.trim()).await
            }).await;
            let _ = this.update(cx, |this, cx| {
                this.addon_busy = false;
                match result {
                    Ok(Ok(results)) => {
                        let count = results.streams.len();
                        let rejected = results.rejected_items;
                        this.addon_stream_results = results.streams;
                        this.addon_result_message = Some(format!("Streams received: {count}; rejected unsafe or malformed entries: {rejected}."));
                    }
                    Ok(Err(error)) => this.addon_error = Some(error.to_string()),
                    Err(error) => this.addon_error = Some(format!("Stream request failed: {error}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn find_addon_subtitles(&mut self, addon_id: String, cx: &mut Context<Self>) {
        if self.addon_busy {
            return;
        }
        let media_type = self.addon_media_type_input.read(cx).value().to_string();
        let media_id = self.addon_media_id_input.read(cx).value().to_string();
        if media_type.trim().is_empty() || media_id.trim().is_empty() {
            self.addon_error =
                Some("Enter a media type and ID before requesting subtitles.".to_owned());
            cx.notify();
            return;
        }
        let extras_raw = self.addon_catalog_extras_input.read(cx).value().to_string();
        let extras = match parse_addon_extras(&extras_raw) {
            Ok(extras) => extras,
            Err(error) => {
                self.addon_error = Some(error);
                cx.notify();
                return;
            }
        };
        let manager = self.addon_manager.clone();
        let runtime = self.tokio.clone();
        self.addon_busy = true;
        self.addon_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime.spawn(async move {
                manager.fetch_subtitles("default", &addon_id, media_type.trim(), media_id.trim(), &extras).await
            }).await;
            let _ = this.update(cx, |this, cx| {
                this.addon_busy = false;
                match result {
                    Ok(Ok(results)) => {
                        let count = results.subtitles.len();
                        let rejected = results.rejected_items;
                        this.addon_subtitle_results = results.subtitles;
                        this.addon_result_message = Some(format!("Subtitles received: {count}; rejected unsafe or malformed entries: {rejected}."));
                    }
                    Ok(Err(error)) => this.addon_error = Some(error.to_string()),
                    Err(error) => this.addon_error = Some(format!("Subtitle request failed: {error}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn addon_control<F, L, I>(
        &self,
        id: I,
        label: L,
        tab_index: isize,
        on_click: F,
    ) -> impl IntoElement + use<F, L, I>
    where
        F: Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
        L: Into<SharedString>,
        I: Into<SharedString>,
    {
        let id: SharedString = id.into();
        let label: SharedString = label.into();
        div()
            .id(id)
            .tab_index(tab_index)
            .rounded_md()
            .border_1()
            .border_color(rgb(Theme::BORDER))
            .bg(rgb(Theme::SURFACE_RAISED))
            .px_3()
            .py_2()
            .cursor_pointer()
            .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
            .hover(|style| style.bg(rgb(Theme::SURFACE_HOVER)))
            .on_click(on_click)
            .child(div().text_xs().text_color(rgb(Theme::ACCENT)).child(label))
    }

    fn render_addons(&self, cx: &mut Context<Self>) -> gpui::Div {
        let url_input = Input::new(&self.addon_url_input)
            .tab_index(55)
            .aria_label("Stremio add-on manifest URL")
            .cleanable(true)
            .w_full()
            .bg(rgb(Theme::SURFACE))
            .border_1()
            .border_color(rgb(Theme::BORDER))
            .rounded_lg()
            .px_4()
            .py_3();
        let media_type_input = Input::new(&self.addon_media_type_input)
            .tab_index(58)
            .aria_label("Media type for stream and subtitle tests")
            .cleanable(true)
            .w_full()
            .bg(rgb(Theme::SURFACE))
            .border_1()
            .border_color(rgb(Theme::BORDER))
            .rounded_lg()
            .px_4()
            .py_3();
        let media_id_input = Input::new(&self.addon_media_id_input)
            .tab_index(59)
            .aria_label("Media identifier for stream and subtitle tests")
            .cleanable(true)
            .w_full()
            .bg(rgb(Theme::SURFACE))
            .border_1()
            .border_color(rgb(Theme::BORDER))
            .rounded_lg()
            .px_4()
            .py_3();
        let extras_input = Input::new(&self.addon_catalog_extras_input)
            .tab_index(60)
            .aria_label("Optional Stremio catalogue extra properties")
            .cleanable(true)
            .w_full()
            .bg(rgb(Theme::SURFACE))
            .border_1()
            .border_color(rgb(Theme::BORDER))
            .rounded_lg()
            .px_4()
            .py_3();

        let mut page = div()
            .w_full()
            .flex()
            .flex_col()
            .gap_5()
            .child(page_intro(
                "EXTEND SENZO",
                "Stremio add-ons",
                "Install and manage compatible catalogues, metadata, stream and subtitle providers. Every provider is opt-in and remains isolated from the application core.",
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(Theme::BORDER))
                    .bg(rgb(Theme::SURFACE))
                    .p_4()
                    .child(div().text_sm().text_color(rgb(Theme::TEXT_PRIMARY)).child("Add an add-on"))
                    .child(url_input)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(self.addon_control(
                                "addon-allow-http",
                                if self.addon_allow_http { "HTTP allowed · less secure" } else { "HTTPS only · recommended" },
                                56,
                                cx.listener(|this, _, _, cx| {
                                    this.addon_allow_http = !this.addon_allow_http;
                                    this.addon_error = None;
                                    cx.notify();
                                }),
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .text_xs()
                                    .text_color(rgb(if self.addon_allow_http { Theme::WARNING } else { Theme::TEXT_MUTED }))
                                    .child(if self.addon_allow_http {
                                        "HTTP can expose the connection. Only use it for a provider you trust."
                                    } else {
                                        "Plain HTTP manifests are rejected unless you explicitly enable the exception."
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .id("addon-preview-button")
                            .tab_index(57)
                            .rounded_md()
                            .bg(rgb(Theme::ACCENT_DARK))
                            .px_4()
                            .py_3()
                            .cursor_pointer()
                            .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                            .on_click(cx.listener(|this, _, _, cx| this.preview_addon(cx)))
                            .child(div().text_sm().text_color(rgb(Theme::TEXT_PRIMARY)).child(if self.addon_busy { "Working…" } else { "Preview manifest" })),
                    ),
            );

        if let Some(preview) = self.addon_preview.as_ref() {
            let resource_names = preview
                .manifest
                .resources
                .iter()
                .map(|resource| resource.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let catalog_count = preview.manifest.catalogs.len();
            page = page.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(Theme::ACCENT))
                    .bg(rgb(Theme::SURFACE_RAISED))
                    .p_4()
                    .child(div().text_sm().text_color(rgb(Theme::ACCENT)).child("MANIFEST PREVIEW"))
                    .child(div().text_lg().text_color(rgb(Theme::TEXT_PRIMARY)).child(preview.manifest.name.clone()))
                    .child(div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child(format!("{} · version {} · {}", preview.manifest.id, preview.manifest.version, preview.display_address)))
                    .child(div().text_sm().text_color(rgb(Theme::TEXT_SECONDARY)).child(preview.manifest.description.clone()))
                    .child(div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child(format!("Resources: {resource_names} · {catalog_count} catalogue(s)")))
                    .child(div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child(if preview.requires_configuration() { "This manifest requires provider-side configuration. Use the provider's configured install URL." } else if preview.manifest.behavior_hints.configurable { "Provider supports configuration; paste its configured manifest URL when credentials/options are required." } else { "No provider configuration is required by the manifest." }))
                    .child(self.addon_control(
                        "addon-confirm-install",
                        "Confirm installation for Default profile",
                        61,
                        cx.listener(|this, _, _, cx| this.install_addon_preview(cx)),
                    )),
            );
        }

        if let Some(error) = self.addon_error.as_deref() {
            page = page.child(status_panel(
                StatusTone::Error,
                "ADD-ON REQUEST FAILED",
                "The provider could not be used",
                error,
            ));
        }
        if let Some(notice) = self.addon_notice.as_deref() {
            page = page.child(status_panel(
                StatusTone::Info,
                "ADD-ON STATUS",
                "Provider update",
                notice,
            ));
        }

        page = page.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(Theme::TEXT_PRIMARY))
                        .child(format!("Installed providers · {}", self.addon_state.len())),
                )
                .child(self.addon_control(
                    "addon-refresh-all",
                    "Refresh all",
                    62,
                    cx.listener(|this, _, _, cx| this.refresh_all_addons(cx)),
                )),
        );

        if self.addon_state.is_empty() {
            page = page.child(status_panel(
                StatusTone::Empty,
                "NO INSTALLED ADD-ONS",
                "Nothing has been installed",
                "Paste a Stremio manifest URL, preview its capabilities, then confirm installation. Torrentio is not bundled; it can only be used when you install its configured manifest yourself.",
            ));
        } else {
            let mut installed = div().w_full().flex().flex_col().gap_3();
            for (index, addon) in self.addon_state.iter().cloned().enumerate() {
                let addon_id = addon.id.clone();
                let mut card = div()
                    .id(format!("installed-addon-{index}"))
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
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div().text_sm().text_color(rgb(Theme::TEXT_PRIMARY)).child(
                                    addon
                                        .manifest
                                        .as_ref()
                                        .map(|manifest| manifest.name.clone())
                                        .unwrap_or_else(|| addon.display_address.clone()),
                                ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(if addon.enabled {
                                        Theme::ACCENT
                                    } else {
                                        Theme::TEXT_MUTED
                                    }))
                                    .child(if addon.enabled { "ENABLED" } else { "DISABLED" }),
                            ),
                    );
                if let Some(manifest) = addon.manifest.as_ref() {
                    card = card.child(div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child(
                        format!(
                            "{} · v{} · {} catalogue(s)",
                            manifest.id,
                            manifest.version,
                            manifest.catalogs.len()
                        ),
                    ));
                    if addon.enabled && !manifest.catalogs.is_empty() {
                        let mut catalogs = div().flex().flex_wrap().gap_2();
                        for (catalog_index, catalog) in manifest.catalogs.iter().enumerate() {
                            let action_id = addon_id.clone();
                            let media_type = catalog.media_type.clone();
                            let catalog_id = catalog.id.clone();
                            let label = format!("Browse {}", catalog.name);
                            catalogs = catalogs.child(self.addon_control(
                                &format!("addon-{index}-catalog-{catalog_index}"),
                                label,
                                70 + catalog_index as isize,
                                cx.listener(move |this, _, _, cx| {
                                    this.browse_addon_catalog(
                                        action_id.clone(),
                                        media_type.clone(),
                                        catalog_id.clone(),
                                        cx,
                                    )
                                }),
                            ));
                        }
                        card = card.child(catalogs);
                    }
                    let supports_meta = addon.enabled
                        && manifest
                            .resources
                            .iter()
                            .any(|resource| resource.name == "meta");
                    if supports_meta {
                        let meta_id = addon_id.clone();
                        card = card.child(self.addon_control(
                            &format!("addon-{index}-metadata"),
                            "Get metadata",
                            93 + index as isize * 3,
                            cx.listener(move |this, _, _, cx| {
                                this.find_addon_metadata(meta_id.clone(), cx)
                            }),
                        ));
                    }
                    let supports_streams = addon.enabled
                        && manifest
                            .resources
                            .iter()
                            .any(|resource| resource.name == "stream");
                    let supports_subtitles = addon.enabled
                        && manifest
                            .resources
                            .iter()
                            .any(|resource| resource.name == "subtitles");
                    if supports_streams || supports_subtitles {
                        let mut actions = div().flex().flex_wrap().gap_2();
                        if supports_streams {
                            let id = addon_id.clone();
                            actions = actions.child(self.addon_control(
                                &format!("addon-{index}-streams"),
                                "Find streams",
                                95 + index as isize * 2,
                                cx.listener(move |this, _, _, cx| {
                                    this.find_addon_streams(id.clone(), cx)
                                }),
                            ));
                        }
                        if supports_subtitles {
                            actions = actions.child(self.addon_control(
                                &format!("addon-{index}-subtitles"),
                                "Find subtitles",
                                96 + index as isize * 2,
                                cx.listener(move |this, _, _, cx| {
                                    this.find_addon_subtitles(addon_id.clone(), cx)
                                }),
                            ));
                        }
                        card = card.child(actions);
                    }
                } else {
                    card =
                        card.child(div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child(
                            addon.last_error.clone().unwrap_or_else(|| {
                                "Manifest not loaded. Refresh to retry.".to_owned()
                            }),
                        ));
                }

                let enable_id = addon.id.clone();
                let remove_id = addon.id.clone();
                let refresh_id = addon.id.clone();
                let move_up_id = addon.id.clone();
                let move_down_id = addon.id.clone();
                card = card.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(self.addon_control(
                            &format!("addon-{index}-toggle"),
                            if addon.enabled { "Disable" } else { "Enable" },
                            110 + index as isize * 3,
                            cx.listener(move |this, _, _, cx| {
                                this.change_addon_enabled(enable_id.clone(), !addon.enabled, cx)
                            }),
                        ))
                        .child(self.addon_control(
                            &format!("addon-{index}-refresh"),
                            "Refresh manifest",
                            111 + index as isize * 3,
                            cx.listener(move |this, _, _, cx| {
                                this.refresh_addon(refresh_id.clone(), cx)
                            }),
                        ))
                        .child(self.addon_control(
                            &format!("addon-{index}-move-up"),
                            "Move up",
                            112 + index as isize * 5,
                            cx.listener(move |this, _, _, cx| {
                                this.move_addon(move_up_id.clone(), -1, cx)
                            }),
                        ))
                        .child(self.addon_control(
                            &format!("addon-{index}-move-down"),
                            "Move down",
                            113 + index as isize * 5,
                            cx.listener(move |this, _, _, cx| {
                                this.move_addon(move_down_id.clone(), 1, cx)
                            }),
                        ))
                        .child(self.addon_control(
                            &format!("addon-{index}-remove"),
                            "Remove",
                            114 + index as isize * 5,
                            cx.listener(move |this, _, _, cx| {
                                this.remove_addon(remove_id.clone(), cx)
                            }),
                        )),
                );
                installed = installed.child(card);
            }
            page = page.child(installed);
        }

        page = page.child(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .rounded_lg()
                .border_1()
                .border_color(rgb(Theme::BORDER))
                .bg(rgb(Theme::SURFACE))
                .p_4()
                .child(div().text_sm().text_color(rgb(Theme::TEXT_PRIMARY)).child("Test stream and subtitle resources"))
                .child(div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child("Use a provider-declared media type and identifier. Torrent info hashes are validated as metadata only; SENZO does not start torrent transport in this phase."))
                .child(media_type_input)
                .child(media_id_input)
                .child(extras_input)
                .child(div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child("Catalogue extras use key=value pairs separated by &. Choose a catalogue above to request it."))
        );

        for (addon_id, catalog_id, items) in self.addon_catalog_results.iter() {
            let title = self
                .addon_state
                .iter()
                .find(|addon| &addon.id == addon_id)
                .and_then(|addon| addon.manifest.as_ref())
                .and_then(|manifest| {
                    manifest
                        .catalogs
                        .iter()
                        .find(|catalog| &catalog.id == catalog_id)
                })
                .map(|catalog| catalog.name.clone())
                .unwrap_or_else(|| catalog_id.clone());
            let mut section = div().flex().flex_col().gap_2().child(
                div()
                    .text_sm()
                    .text_color(rgb(Theme::TEXT_PRIMARY))
                    .child(format!("{title} · {} title(s)", items.len())),
            );
            for (index, item) in items.iter().take(30).enumerate() {
                section = section.child(
                    div()
                        .text_sm()
                        .text_color(rgb(Theme::TEXT_SECONDARY))
                        .child(format!(
                            "{}. {} · {} · {}",
                            index + 1,
                            item.display_title(),
                            item.media_type,
                            item.id
                        )),
                );
            }
            page = page.child(section);
        }

        if let Some(meta) = self.addon_meta_result.as_ref() {
            let title = meta
                .get("name")
                .or_else(|| meta.get("title"))
                .and_then(Value::as_str)
                .unwrap_or("Metadata");
            let description = meta
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("No description was provided.");
            page = page.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(Theme::BORDER))
                    .bg(rgb(Theme::SURFACE))
                    .p_4()
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_PRIMARY))
                            .child(title.to_owned()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child(description.to_owned()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(Theme::TEXT_MUTED))
                            .child(format!(
                                "{} · {}",
                                meta.get("type")
                                    .and_then(Value::as_str)
                                    .unwrap_or("unknown type"),
                                meta.get("id")
                                    .and_then(Value::as_str)
                                    .unwrap_or("unknown ID")
                            )),
                    ),
            );
        }
        if let Some(message) = self.addon_result_message.as_deref() {
            page = page.child(status_panel(
                StatusTone::Info,
                "RESOURCE RESPONSE",
                "Provider request completed",
                message,
            ));
        }
        if !self.addon_stream_results.is_empty() {
            let mut streams = div().flex().flex_col().gap_2().child(
                div()
                    .text_sm()
                    .text_color(rgb(Theme::TEXT_PRIMARY))
                    .child(format!("Streams · {}", self.addon_stream_results.len())),
            );
            for (index, stream) in self.addon_stream_results.iter().take(25).enumerate() {
                streams = streams.child(
                    div()
                        .text_xs()
                        .text_color(rgb(Theme::TEXT_SECONDARY))
                        .child(format!(
                            "{}. {} {}",
                            index + 1,
                            stream.name.as_deref().unwrap_or("Stream"),
                            stream
                                .title
                                .as_deref()
                                .or(stream.description.as_deref())
                                .unwrap_or_default()
                        )),
                );
            }
            page = page.child(streams);
        }
        if !self.addon_subtitle_results.is_empty() {
            let mut subtitles = div().flex().flex_col().gap_2().child(
                div()
                    .text_sm()
                    .text_color(rgb(Theme::TEXT_PRIMARY))
                    .child(format!("Subtitles · {}", self.addon_subtitle_results.len())),
            );
            for (index, subtitle) in self.addon_subtitle_results.iter().take(25).enumerate() {
                subtitles = subtitles.child(
                    div()
                        .text_xs()
                        .text_color(rgb(Theme::TEXT_SECONDARY))
                        .child(format!(
                            "{}. {} · {}",
                            index + 1,
                            subtitle.language,
                            subtitle.label.as_deref().unwrap_or("Subtitle")
                        )),
                );
            }
            page = page.child(subtitles);
        }

        page
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
            page = page.child(status_panel(
                StatusTone::Info,
                "READY WHEN YOU ARE",
                "Find something to watch",
                "Enter a title, series, or person's name. Search uses SENZO's hosted metadata service; no TMDB credential is stored in this app.",
            ));
            if !self.recent_searches.is_empty() {
                let mut history = div().w_full().flex().flex_col().gap_2().child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(Theme::TEXT_PRIMARY))
                                .child("Recent searches"),
                        )
                        .child(
                            div()
                                .id("clear-search-history")
                                .tab_index(30)
                                .cursor_pointer()
                                .rounded_md()
                                .px_2()
                                .py_1()
                                .focus_visible(|style| {
                                    style.border_1().border_color(rgb(Theme::ACCENT))
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.clear_search_history();
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(Theme::ACCENT))
                                        .child("Clear"),
                                ),
                        ),
                );
                let mut recent = div().flex().flex_wrap().gap_2();
                for (index, query) in self.recent_searches.iter().cloned().enumerate() {
                    let search_query = query.clone();
                    recent = recent.child(
                        div()
                            .id(format!("recent-search-{index}"))
                            .tab_index(31 + index as isize)
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(Theme::BORDER))
                            .bg(rgb(Theme::SURFACE))
                            .px_3()
                            .py_2()
                            .cursor_pointer()
                            .focus_visible(|style| {
                                style.border_1().border_color(rgb(Theme::ACCENT))
                            })
                            .aria_label(format!("Search again for {query}"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.search_input.update(cx, |input, cx| {
                                    input.set_value(search_query.clone(), window, cx)
                                });
                                this.update_search_query(search_query.clone(), cx);
                            }))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(Theme::TEXT_SECONDARY))
                                    .child(query),
                            ),
                    );
                }
                history = history.child(recent);
                page = page.child(history);
            }
            return page;
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
                                format!(
                                    "Showing {} of {total} · Page {} of {}",
                                    self.search_results.len(),
                                    self.search_page,
                                    self.search_total_pages
                                ),
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

                let mut pager = div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3();
                if self.search_page > 1 {
                    pager = pager.child(
                        div()
                            .id("search-previous-page")
                            .tab_index(70)
                            .cursor_pointer()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(Theme::BORDER))
                            .bg(rgb(Theme::SURFACE))
                            .px_3()
                            .py_2()
                            .aria_label("Previous search results page")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_search_page(this.search_page.saturating_sub(1), cx)
                            }))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(Theme::TEXT_SECONDARY))
                                    .child("Previous"),
                            ),
                    );
                }
                if self.search_page < self.search_total_pages {
                    pager = pager.child(
                        div()
                            .id("search-next-page")
                            .tab_index(71)
                            .cursor_pointer()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(Theme::BORDER))
                            .bg(rgb(Theme::SURFACE))
                            .px_3()
                            .py_2()
                            .aria_label("Next search results page")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_search_page(this.search_page.saturating_add(1), cx)
                            }))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(Theme::TEXT_SECONDARY))
                                    .child("Next"),
                            ),
                    );
                }
                page = page.child(pager);
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
                let focus_id = home_card_focus_id(key, card.media_type, card.id);
                let focus = self.home_card_focus_handles.get(&focus_id);
                let mut card_element = div()
                    .id(focus_id.clone())
                    .tab_index(tab_start + index as isize)
                    .cursor_pointer()
                    .rounded_lg()
                    .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                    .aria_label(format!("Open {}", card.title));
                if let Some(focus) = focus {
                    card_element = card_element.track_focus(focus);
                }
                row = row.child(
                    card_element
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_home_card(
                                id,
                                media_type,
                                media_title.clone(),
                                poster.clone(),
                                focus_id.clone(),
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
                    .debug_selector(|| format!("home-{}-shelf-scroll", key.as_str()))
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

        // Collections are local-first. The Library domain owns collection creation;
        // Home only projects saved rows and never inserts sample data.
        for collection in &self.home_collections {
            content = content.child(home_collection_row(
                collection,
                &self.home_collection_artwork,
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
            .debug_selector(|| format!("navigation-{}", page.key()))
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
            .debug_selector(|| "senzo-sidebar".to_owned())
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

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let page = self.navigation.current();
        div()
            .id("senzo-header")
            .debug_selector(|| "senzo-header".to_owned())
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

    fn load_home_progress_artwork(&mut self, cx: &mut Context<Self>) {
        if self.home_progress_artwork_requested {
            return;
        }
        let requests = self.home_continue_watching.iter()
            .filter_map(|progress| {
                let path = progress.still_path.clone()?;
                let key = playback_progress_artwork_key(progress);
                (!self.home_continue_watching_artwork.contains_key(&key))
                    .then_some((key, path))
            })
            .collect::<Vec<_>>();
        if requests.is_empty() {
            return;
        }
        self.home_progress_artwork_requested = true;
        let artwork = self.artwork.clone();
        let task = self.tokio.spawn(async move {
            let mut jobs = JoinSet::new();
            for (key, path) in requests {
                let service = artwork.clone();
                jobs.spawn(async move {
                    let image = load_gpui_artwork(service, path, ArtworkSize::W780, CancellationToken::new()).await;
                    (key, image)
                });
            }
            let mut loaded = Vec::new();
            while let Some(result) = jobs.join_next().await {
                if let Ok((key, Some(image))) = result {
                    loaded.push((key, image));
                }
            }
            loaded
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.home_progress_artwork_requested = false;
                if let Ok(images) = result {
                    for (key, image) in images {
                        this.home_continue_watching_artwork.insert(key, image);
                    }
                }
                cx.notify();
            });
        }).detach();
    }

    fn load_home_up_next(&mut self, cx: &mut Context<Self>) {
        if self.home_up_next_requested || self.smoke_test {
            return;
        }
        self.home_up_next_requested = true;
        self.home_up_next_loading = true;
        let repository = self.playback_progress_repository.clone();
        let metadata = self.metadata.clone();
        let artwork = self.artwork.clone();
        let cancellation = self.home_cancellation.child_token();
        let task_token = cancellation.clone();
        let task = self.tokio.spawn(async move {
            let completed = tokio::task::spawn_blocking(move || {
                repository.list_recent_completed_series("default", 20)
            }).await.map_err(|error| error.to_string())?
                .map_err(|error| error.to_string())?;
            let mut jobs = JoinSet::new();
            for progress in completed.into_iter().take(16) {
                let metadata = metadata.clone();
                let artwork = artwork.clone();
                let token = task_token.child_token();
                jobs.spawn(async move {
                    let show_id = progress.media_id.parse::<u64>().ok()?;
                    let tv = metadata.tv(show_id, &token).await.ok()?;
                    let mut season_numbers = tv.seasons.iter()
                        .map(|season| season.season_number)
                        .filter(|number| *number > 0)
                        .collect::<Vec<_>>();
                    season_numbers.sort_unstable();
                    season_numbers.dedup();
                    let completed_season = progress.season_number.unwrap_or(0);
                    let completed_episode = progress.episode_number.unwrap_or(0);
                    for season_number in season_numbers.into_iter().filter(|number| *number >= completed_season) {
                        if token.is_cancelled() { return None; }
                        let season = metadata.season(show_id, season_number, &token).await.ok()?;
                        let mut episodes = season.episodes.iter()
                            .filter(|episode| season_number != completed_season || episode.episode_number > completed_episode)
                            .collect::<Vec<_>>();
                        episodes.sort_by_key(|episode| episode.episode_number);
                        if let Some(episode) = episodes.first() {
                            let still = match episode.still_path.as_deref() {
                                Some(path) => load_gpui_artwork(
                                    artwork.clone(),
                                    path.to_owned(),
                                    ArtworkSize::W780,
                                    token.child_token(),
                                ).await,
                                None => None,
                            };
                            return Some(HomeUpNextCard {
                                parent_progress: progress,
                                season_number,
                                episode_number: episode.episode_number,
                                episode_title: episode.name.clone(),
                                air_date: episode.air_date.clone(),
                                still,
                            });
                        }
                    }
                    None
                });
            }
            let mut cards = Vec::new();
            while let Some(result) = jobs.join_next().await {
                if task_token.is_cancelled() { return Ok::<_, String>(Vec::new()); }
                if let Ok(Some(card)) = result { cards.push(card); }
            }
            let today = current_utc_date();
            cards.sort_by(|left, right| {
                let left_date = left.air_date.as_deref().filter(|date| date.len() >= 10).unwrap_or("9999-99-99");
                let right_date = right.air_date.as_deref().filter(|date| date.len() >= 10).unwrap_or("9999-99-99");
                let left_future = left_date > today.as_str();
                let right_future = right_date > today.as_str();
                left_future.cmp(&right_future)
                    .then_with(|| left_date.cmp(right_date))
                    .then_with(|| right.parent_progress.updated_at_ms.cmp(&left.parent_progress.updated_at_ms))
            });
            Ok::<_, String>(cards)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if cancellation.is_cancelled() { return; }
                this.home_up_next_loading = false;
                match result {
                    Ok(Ok(cards)) => this.home_up_next = cards,
                    Ok(Err(error)) => {
                        tracing::warn!(error = %error, "could not enrich the Home Up Next shelf");
                        this.home_up_next = Vec::new();
                    }
                    Err(error) => {
                        tracing::warn!(error = %error, "Home Up Next task failed");
                        this.home_up_next = Vec::new();
                    }
                }
                cx.notify();
            });
        }).detach();
    }

    fn render_continue_watching(&self, cx: &mut Context<Self>) -> gpui::Div {
        let mut cards = div().flex().gap_4();
        for (index, entry) in self.home_continue_watching.iter().enumerate() {
            let progress = entry.clone();
            let position = format_playback_time(entry.position_ms as f64 / 1000.0);
            let duration = format_playback_time(entry.duration_ms as f64 / 1000.0);
            let episode_label = match (entry.season_number, entry.episode_number) {
                (Some(season), Some(episode)) => format!("S{season:02} E{episode:02}"),
                _ => "MOVIE".to_owned(),
            };
            let detail = format!(
                "{} · {}{}",
                episode_label,
                position,
                if entry.duration_ms > 0 { format!(" / {duration}") } else { String::new() }
            );
            let artwork = self.home_continue_watching_artwork
                .get(&playback_progress_artwork_key(entry))
                .cloned();
            cards = cards.child(
                div()
                    .id(format!("continue-watching-{}", index))
                    .tab_index(120 + index as isize)
                    .cursor_pointer()
                    .rounded_lg()
                    .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                    .aria_label(format!("Continue watching {}", entry.title))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_continue_watching(progress.clone(), cx);
                    }))
                    .child(poster_card(&entry.title, &detail, artwork, false)),
            );
        }
        let mut sections = div().w_full().flex().flex_col().gap_5();
        if !self.home_continue_watching.is_empty() {
            sections = sections.child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().text_lg().text_color(rgb(Theme::TEXT_PRIMARY)).child("Continue Watching"))
                    .child(div().id("home-continue-watching-scroll").w_full().overflow_x_scroll().child(cards)),
            );
        }
        if !self.home_up_next.is_empty() {
            let today = current_utc_date();
            let mut up_next_cards = div().flex().gap_4();
            for (index, card) in self.home_up_next.iter().enumerate() {
                let mut progress = card.parent_progress.clone();
                progress.season_number = Some(card.season_number);
                progress.episode_number = Some(card.episode_number);
                progress.episode_title = Some(card.episode_title.clone());
                progress.still_path = progress.still_path.clone();
                progress.position_ms = 0;
                progress.duration_ms = 0;
                progress.is_completed = false;
                let date_label = card.air_date.as_deref()
                    .filter(|date| date.len() >= 10)
                    .map(|date| if date > today.as_str() { format!("COMING · {date}") } else { format!("UP NEXT · {date}") })
                    .unwrap_or_else(|| "UP NEXT".to_owned());
                let detail = format!("S{:02} E{:02} · {}",
                    card.season_number, card.episode_number, date_label);
                up_next_cards = up_next_cards.child(
                    div()
                        .id(format!("home-up-next-{index}"))
                        .tab_index(160 + index as isize)
                        .cursor_pointer()
                        .rounded_lg()
                        .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                        .aria_label(format!("Open next episode {}", card.episode_title))
                        .on_click(cx.listener(move |this, _, _, cx| this.open_continue_watching(progress.clone(), cx)))
                        .child(poster_card(
                            &card.parent_progress.title,
                            &detail,
                            card.still.clone(),
                            false,
                        )),
                );
            }
            sections = sections.child(
                div().w_full().flex().flex_col().gap_3()
                    .child(div().text_lg().text_color(rgb(Theme::TEXT_PRIMARY)).child("Up Next"))
                    .child(div().id("home-up-next-scroll").w_full().overflow_x_scroll().child(up_next_cards)),
            );
        } else if self.home_up_next_loading {
            sections = sections.child(
                div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child("Finding next episodes from your watched series…"),
            );
        }
        sections
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
        .debug_selector(|| "home-featured-hero".to_owned())
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
                    .track_focus(
                        self.home_card_focus_handles
                            .get("home-featured-more-info")
                            .expect("featured focus handle exists"),
                    )
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
                            "home-featured-more-info".to_owned(),
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
                    .debug_selector(|| "home-refresh-catalogues".to_owned())
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
        if !self.home_continue_watching.is_empty() {
            page = page.child(self.render_continue_watching(cx));
        }
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

    fn connect_debrid_provider(&mut self, cx: &mut Context<Self>) {
        if self.source_settings_busy {
            return;
        }
        let provider_raw = self
            .debrid_provider_input
            .read(cx)
            .value()
            .trim()
            .to_owned();
        let api_key = self.debrid_api_key_input.read(cx).value().trim().to_owned();
        if api_key.is_empty() {
            self.source_settings_error = Some("Enter a provider API key first.".to_owned());
            cx.notify();
            return;
        }
        let provider = match DebridProviderId::parse(&provider_raw) {
            Ok(provider) => provider,
            Err(error) => {
                self.source_settings_error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let manager = self.debrid_manager.clone();
        let runtime = self.tokio.clone();
        self.source_settings_busy = true;
        self.source_settings_error = None;
        self.source_settings_notice = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let task = runtime.spawn(async move { manager.connect("default", provider, &api_key).await });
            let result = task.await.map_err(|error| error.to_string()).and_then(|value| value.map_err(|error| error.to_string()));
            let _ = this.update(cx, |this, cx| {
                this.source_settings_busy = false;
                match result {
                    Ok(account) => this.source_settings_notice = Some(format!("{} connected. The API key is stored in the operating-system credential store.", account.provider.name())),
                    Err(error) => this.source_settings_error = Some(error),
                }
                cx.notify();
            });
        }).detach();
    }

    fn disconnect_debrid_provider(&mut self, provider: DebridProviderId, cx: &mut Context<Self>) {
        match self.debrid_manager.disconnect("default", provider) {
            Ok(()) => {
                self.source_settings_notice = Some(format!("{} disconnected.", provider.name()))
            }
            Err(error) => self.source_settings_error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn connect_media_server(&mut self, cx: &mut Context<Self>) {
        if self.source_settings_busy {
            return;
        }
        let provider_raw = self
            .server_provider_input
            .read(cx)
            .value()
            .trim()
            .to_owned();
        let name = self.server_name_input.read(cx).value().trim().to_owned();
        let address = self.server_address_input.read(cx).value().trim().to_owned();
        let username = self
            .server_username_input
            .read(cx)
            .value()
            .trim()
            .to_owned();
        let password = self.server_password_input.read(cx).value().to_string();
        let provider = match MediaServerProvider::parse(&provider_raw) {
            Ok(provider) => provider,
            Err(error) => {
                self.source_settings_error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let manager = self.media_server_manager.clone();
        let runtime = self.tokio.clone();
        self.source_settings_busy = true;
        self.source_settings_error = None;
        self.source_settings_notice = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let task = runtime.spawn(async move {
                manager
                    .connect("default", provider, &name, &address, &username, &password)
                    .await
            });
            let result = task
                .await
                .map_err(|error| error.to_string())
                .and_then(|value| value.map_err(|error| error.to_string()));
            let _ = this.update(cx, |this, cx| {
                this.source_settings_busy = false;
                match result {
                    Ok(connection) => {
                        this.source_settings_notice = Some(format!(
                            "{} connected. {} library/libraries discovered.",
                            connection.provider.display_name(),
                            connection.libraries.len()
                        ))
                    }
                    Err(error) => this.source_settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn remove_media_server(&mut self, connection_id: String, cx: &mut Context<Self>) {
        match self.media_server_manager.remove("default", &connection_id) {
            Ok(()) => {
                self.source_settings_notice = Some(
                    "Media-server connection removed and its saved credential deleted.".to_owned(),
                )
            }
            Err(error) => self.source_settings_error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn render_source_provider_settings(&self, cx: &mut Context<Self>) -> gpui::Div {
        let debrid_provider = Input::new(&self.debrid_provider_input)
            .tab_index(110)
            .aria_label("Debrid provider ID")
            .w_full()
            .bg(rgb(Theme::SURFACE))
            .border_1()
            .border_color(rgb(Theme::BORDER))
            .rounded_lg()
            .px_3()
            .py_2();
        let debrid_key = Input::new(&self.debrid_api_key_input)
            .tab_index(111)
            .aria_label("Debrid API key")
            .cleanable(true)
            .w_full()
            .bg(rgb(Theme::SURFACE))
            .border_1()
            .border_color(rgb(Theme::BORDER))
            .rounded_lg()
            .px_3()
            .py_2();
        let server_provider = Input::new(&self.server_provider_input)
            .tab_index(114)
            .aria_label("Media server provider")
            .w_full()
            .bg(rgb(Theme::SURFACE))
            .border_1()
            .border_color(rgb(Theme::BORDER))
            .rounded_lg()
            .px_3()
            .py_2();
        let server_name = Input::new(&self.server_name_input)
            .tab_index(115)
            .aria_label("Media server display name")
            .w_full()
            .bg(rgb(Theme::SURFACE))
            .border_1()
            .border_color(rgb(Theme::BORDER))
            .rounded_lg()
            .px_3()
            .py_2();
        let server_address = Input::new(&self.server_address_input)
            .tab_index(116)
            .aria_label("Media server URL")
            .w_full()
            .bg(rgb(Theme::SURFACE))
            .border_1()
            .border_color(rgb(Theme::BORDER))
            .rounded_lg()
            .px_3()
            .py_2();
        let server_username = Input::new(&self.server_username_input)
            .tab_index(117)
            .aria_label("Media server username")
            .w_full()
            .bg(rgb(Theme::SURFACE))
            .border_1()
            .border_color(rgb(Theme::BORDER))
            .rounded_lg()
            .px_3()
            .py_2();
        let server_password = Input::new(&self.server_password_input)
            .tab_index(118)
            .aria_label("Media server password")
            .cleanable(true)
            .w_full()
            .bg(rgb(Theme::SURFACE))
            .border_1()
            .border_color(rgb(Theme::BORDER))
            .rounded_lg()
            .px_3()
            .py_2();
        let mut panel = div().w_full().flex().flex_col().gap_3()
            .rounded_lg().border_1().border_color(rgb(Theme::BORDER)).bg(rgb(Theme::SURFACE)).p_4()
            .child(div().text_sm().text_color(rgb(Theme::ACCENT)).child("SOURCE PROVIDERS"))
            .child(div().text_lg().text_color(rgb(Theme::TEXT_PRIMARY)).child("Debrid services"))
            .child(div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child("Connect Torbox, Premiumize or Real-Debrid with an API key. Keys are stored in the operating-system credential store, never in SQLite."))
            .child(debrid_provider).child(debrid_key)
            .child(self.addon_control("source-connect-debrid", if self.source_settings_busy { "Connecting…" } else { "Connect debrid provider" }, 112,
                cx.listener(|this, _, _, cx| this.connect_debrid_provider(cx))));
        if let Ok(accounts) = self.debrid_manager.accounts("default") {
            for account in accounts {
                let provider = account.provider;
                panel = panel.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .rounded_md()
                        .bg(rgb(Theme::SURFACE_RAISED))
                        .p_2()
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .text_color(rgb(Theme::TEXT_SECONDARY))
                                .child(if account.configured {
                                    format!(
                                        "{} · Connected · {}",
                                        provider.name(),
                                        account.masked_credential.unwrap_or_default()
                                    )
                                } else {
                                    format!("{} · Not connected", provider.name())
                                }),
                        )
                        .child(self.addon_control(
                            &format!("source-disconnect-{}", provider.id()),
                            "Disconnect",
                            113,
                            cx.listener(move |this, _, _, cx| {
                                this.disconnect_debrid_provider(provider, cx)
                            }),
                        )),
                );
            }
        }
        panel = panel
            .child(div().w_full().h(px(1.0)).bg(rgb(Theme::BORDER)))
            .child(div().text_lg().text_color(rgb(Theme::TEXT_PRIMARY)).child("Jellyfin & Emby"))
            .child(div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child("Connect a server on your local network or a trusted remote address. Only explicitly configured server hosts can use private LAN destinations. The password is placed in the OS credential store after a successful sign-in."))
            .child(server_provider).child(server_name).child(server_address).child(server_username).child(server_password)
            .child(self.addon_control("source-connect-media-server", if self.source_settings_busy { "Connecting…" } else { "Connect media server" }, 119,
                cx.listener(|this, _, _, cx| this.connect_media_server(cx))));
        if let Ok(connections) = self.media_server_manager.list_connections("default") {
            if connections.is_empty() {
                panel = panel.child(status_panel(StatusTone::Empty, "NO MEDIA SERVERS", "No Jellyfin or Emby connections", "Connected libraries will be available as independent sources in movie and episode Details."));
            } else {
                for connection in connections {
                    let id = connection.id.clone();
                    panel = panel.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .rounded_md()
                            .bg(rgb(Theme::SURFACE_RAISED))
                            .p_2()
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div().text_sm().text_color(rgb(Theme::TEXT_PRIMARY)).child(
                                            format!(
                                                "{} · {}",
                                                connection.provider.display_name(),
                                                connection.name
                                            ),
                                        ),
                                    )
                                    .child(
                                        div().text_xs().text_color(rgb(Theme::TEXT_MUTED)).child(
                                            format!(
                                                "{} · {} library/libraries",
                                                connection.address,
                                                connection.libraries.len()
                                            ),
                                        ),
                                    ),
                            )
                            .child(self.addon_control(
                                &format!("source-remove-server-{id}"),
                                "Remove",
                                120,
                                cx.listener(move |this, _, _, cx| {
                                    this.remove_media_server(id.clone(), cx)
                                }),
                            )),
                    );
                }
            }
        }
        if let Some(notice) = self.source_settings_notice.as_deref() {
            panel = panel.child(
                div()
                    .text_xs()
                    .text_color(rgb(Theme::SUCCESS))
                    .child(notice.to_owned()),
            );
        }
        if let Some(error) = self.source_settings_error.as_deref() {
            panel = panel.child(
                div()
                    .text_xs()
                    .text_color(rgb(Theme::DANGER))
                    .child(error.to_owned()),
            );
        }
        panel
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
            .child(self.render_plugin_settings(cx))
            .child(self.render_source_provider_settings(cx))
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
            AppPage::Movies => self.render_discovery(MediaType::Movie, cx),
            AppPage::TvShows => self.render_discovery(MediaType::Tv, cx),
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
            AppPage::Addons => self.render_addons(cx),
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

async fn load_home_shelf_initial(
    definition: HomeCatalogDefinition,
    metadata: MetadataClient,
    artwork: ArtworkService,
    is_hero_source: bool,
    cancellation: CancellationToken,
) -> Option<HomeShelfInitialResult> {
    let key = definition.built_in_catalog()?;
    let descriptor_identity = definition.cache_identity();
    let response = match key {
        HomeShelfKey::Trending => metadata.trending(1, "en-US", &cancellation).await,
        HomeShelfKey::PopularMovies => metadata.popular_movies(1, "en-US", &cancellation).await,
        HomeShelfKey::PopularTv => metadata.popular_tv(1, "en-US", &cancellation).await,
    };

    if cancellation.is_cancelled() {
        return None;
    }

    let mut data = match response {
        Ok(response) => {
            let page = response.page.clamp(1, MAX_HOME_CATALOGUE_PAGES);
            let fallback_type = match key {
                HomeShelfKey::Trending => None,
                HomeShelfKey::PopularMovies => Some(MediaType::Movie),
                HomeShelfKey::PopularTv => Some(MediaType::Tv),
            };
            home_shelf_load_data(
                home_media_candidates(response.results, fallback_type),
                None,
                page,
                response.total_pages,
            )
        }
        Err(error) => {
            home_shelf_load_data(Vec::new(), Some(metadata_error_message(&error)), 0, None)
        }
    };

    let featured_indices = if is_hero_source {
        data.candidates
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| candidate.backdrop_path.as_ref().map(|_| index))
            .take(MAX_HOME_FEATURED_ITEMS)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let featured_index_set = featured_indices.iter().copied().collect::<HashSet<_>>();

    let mut jobs = JoinSet::new();
    for (index, candidate) in data.candidates.iter().enumerate() {
        let service = artwork.clone();
        let token = cancellation.child_token();
        let poster_path = candidate.poster_path.clone();
        let backdrop_path = if featured_index_set.contains(&index) {
            candidate.backdrop_path.clone()
        } else {
            None
        };
        jobs.spawn(async move {
            let poster = load_gpui_artwork(
                service.clone(),
                poster_path,
                ArtworkSize::W342,
                token.child_token(),
            )
            .await;
            let backdrop = match backdrop_path {
                Some(path) => {
                    load_gpui_artwork(service, path, ArtworkSize::W1280, token.child_token()).await
                }
                None => None,
            };
            (index, poster, backdrop)
        });
    }

    let mut backdrops = HashMap::new();
    while let Some(result) = jobs.join_next().await {
        if cancellation.is_cancelled() {
            return None;
        }
        if let Ok((index, poster, backdrop)) = result
            && let Some(card) = data.cards.get_mut(index)
        {
            card.poster = poster;
            if let Some(backdrop) = backdrop {
                backdrops.insert(index, backdrop);
            }
        }
    }

    if cancellation.is_cancelled() {
        return None;
    }

    let featured = featured_indices
        .into_iter()
        .filter_map(|index| {
            let candidate = data.candidates.get(index)?;
            let backdrop = backdrops.remove(&index)?;
            let poster = data.cards.get(index).and_then(|card| card.poster.clone());
            Some(HomeFeaturedCard {
                id: candidate.id,
                media_type: candidate.media_type,
                title: candidate.title.clone(),
                detail: candidate.detail.clone(),
                overview: candidate.overview.clone(),
                poster,
                backdrop,
            })
        })
        .collect();

    Some(HomeShelfInitialResult {
        key,
        descriptor_identity,
        data,
        featured,
    })
}

fn home_collection_artwork_requests(collections: &[HomeCollection]) -> Vec<(String, String)> {
    visible_home_collections(collections)
        .into_iter()
        .flat_map(|collection| {
            let collection_id = collection.id;
            collection.folders.into_iter().filter_map(move |folder| {
                let path = folder.cover_artwork_path?;
                let key = home_collection_folder_key(&collection_id, &folder.id)?;
                Some((key, path))
            })
        })
        .take(MAX_HOME_COLLECTION_ARTWORK_ITEMS)
        .collect()
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
        self.home_collection_artwork_cancellation.cancel();
        self.search_cancellation.cancel();
        self.details_cancellation.cancel();
        self.person_details_cancellation.cancel();
        self.playback_controls_timer.cancel();
    }
}

impl Render for SenzoShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.smoke_test && self.navigation.current() == AppPage::Home {
            self.load_home_progress_artwork(cx);
            self.load_home_up_next(cx);
            self.load_home_collection_artwork(cx);
            if self.home_catalogue_status == HomeCatalogueStatus::NotRequested {
                self.load_home_catalogue(cx);
            }
        }

        if self.smoke_test && !cfg!(test) && !self.smoke_test_scheduled {
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
            .on_action(cx.listener(|this, _: &ToggleFullscreen, window, cx| {
                if this.playback_open {
                    this.toggle_player_fullscreen(cx);
                } else {
                    window.toggle_fullscreen();
                }
            }))
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
            .on_action(cx.listener(|this, _: &PlayerTogglePlayback, _window, cx| {
                this.toggle_pause_playback(cx);
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerSeekBack, _window, cx| {
                this.seek_playback(-10.0, cx);
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerSeekForward, _window, cx| {
                this.seek_playback(10.0, cx);
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerVolumeDown, _window, cx| {
                this.adjust_playback_volume(-5.0, cx);
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerVolumeUp, _window, cx| {
                this.adjust_playback_volume(5.0, cx);
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerToggleMute, _window, cx| {
                this.toggle_playback_mute(cx);
                this.reveal_playback_controls(cx);
            }))
            .on_action(
                cx.listener(|this, _: &PlayerToggleFullscreen, _window, cx| {
                    this.toggle_player_fullscreen(cx);
                    this.reveal_playback_controls(cx);
                }),
            )
            .on_action(cx.listener(|this, _: &PlayerCycleResize, _window, cx| {
                this.cycle_player_resize_mode(cx);
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerShowAudio, _window, cx| {
                this.playback_panel = Some(PlayerPanel::Audio);
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerShowSubtitles, _window, cx| {
                this.playback_panel = Some(PlayerPanel::Subtitles);
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerShowChapters, _window, cx| {
                this.playback_panel = Some(PlayerPanel::Chapters);
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerShowInfo, _window, cx| {
                this.playback_panel = Some(PlayerPanel::StreamInfo);
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerShowSources, _window, cx| {
                this.playback_panel = Some(PlayerPanel::Sources);
                this.reveal_playback_controls(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerRetry, _window, cx| {
                this.retry_playback(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerNextSource, _window, cx| {
                this.try_next_playback_source(cx);
            }))
            .on_action(cx.listener(|this, _: &PlayerTogglePip, _window, cx| {
                this.toggle_player_picture_in_picture(cx);
                this.reveal_playback_controls(cx);
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
                    // A row flex item defaults to its min-content width. Home
                    // shelves and hero content must not force this main column
                    // wider than the space left after the fixed sidebar.
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .child(header)
                    .child(
                        div()
                            .id("page-content")
                            .debug_selector(|| "page-content".to_owned())
                            .track_scroll(&self.page_scroll_handle)
                            .flex_1()
                            .min_w(px(0.0))
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
        if self.playback_open {
            root = root.child(self.render_player_screen(window, cx));
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
        KeyBinding::new("space", PlayerTogglePlayback, Some("SENZO_PLAYER")),
        KeyBinding::new("left", PlayerSeekBack, Some("SENZO_PLAYER")),
        KeyBinding::new("right", PlayerSeekForward, Some("SENZO_PLAYER")),
        KeyBinding::new("down", PlayerVolumeDown, Some("SENZO_PLAYER")),
        KeyBinding::new("up", PlayerVolumeUp, Some("SENZO_PLAYER")),
        KeyBinding::new("m", PlayerToggleMute, Some("SENZO_PLAYER")),
        KeyBinding::new("f", PlayerToggleFullscreen, Some("SENZO_PLAYER")),
        KeyBinding::new("r", PlayerCycleResize, Some("SENZO_PLAYER")),
        KeyBinding::new("a", PlayerShowAudio, Some("SENZO_PLAYER")),
        KeyBinding::new("s", PlayerShowSubtitles, Some("SENZO_PLAYER")),
        KeyBinding::new("c", PlayerShowChapters, Some("SENZO_PLAYER")),
        KeyBinding::new("i", PlayerShowInfo, Some("SENZO_PLAYER")),
        KeyBinding::new("o", PlayerShowSources, Some("SENZO_PLAYER")),
        KeyBinding::new("[", PlayerPreviousEpisode, Some("SENZO_PLAYER")),
        KeyBinding::new("]", PlayerNextEpisode, Some("SENZO_PLAYER")),
        KeyBinding::new("p", PlayerTogglePip, Some("SENZO_PLAYER")),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("ctrl-q", Quit, None),
        KeyBinding::new("cmd-shift-/", ShowShortcuts, None),
    ]);
}

pub fn run(services: ShellServices, smoke_test: bool) {
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
                let shell = cx.new(|cx| SenzoShell::new(services, smoke_test, window, cx));
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
    use senzo_catalog::{HomeCollectionFolder, HomeCollectionPosterShape};
    use senzo_domain::{RepositoryError, SettingKey};

    struct TestSettingsRepository;
    #[derive(Default)]
    struct TestAddonStore;

    impl senzo_addons::AddonStore for TestAddonStore {
        fn list(
            &self,
            _profile_id: &str,
        ) -> Result<Vec<senzo_addons::AddonRecord>, senzo_addons::AddonStoreError> {
            Ok(Vec::new())
        }
        fn insert(
            &self,
            _profile_id: &str,
            _record: &senzo_addons::AddonRecord,
        ) -> Result<(), senzo_addons::AddonStoreError> {
            Ok(())
        }
        fn update(
            &self,
            _profile_id: &str,
            _record: &senzo_addons::AddonRecord,
        ) -> Result<(), senzo_addons::AddonStoreError> {
            Ok(())
        }
        fn remove(
            &self,
            _profile_id: &str,
            _id: &str,
        ) -> Result<(), senzo_addons::AddonStoreError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct TestAddonSecrets;

    impl senzo_addons::AddonSecretStore for TestAddonSecrets {
        fn get(&self, _key: &str) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn set(&self, _key: &str, _secret: &str) -> Result<(), String> {
            Ok(())
        }
        fn delete(&self, _key: &str) -> Result<(), String> {
            Ok(())
        }
    }

    fn test_addon_manager() -> AddonManager {
        AddonManager::new(Arc::new(TestAddonStore), Arc::new(TestAddonSecrets))
    }

    #[derive(Default)]
    struct TestPluginSettings;

    impl senzo_plugins::PluginSettingsStore for TestPluginSettings {
        fn get(&self, _account: &str) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn set(&self, _account: &str, _value: &str) -> Result<(), String> {
            Ok(())
        }
        fn delete(&self, _account: &str) -> Result<(), String> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct TestDebridCredentials;

    impl senzo_debrid::DebridCredentialStore for TestDebridCredentials {
        fn get(&self, _account: &str) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn set(&self, _account: &str, _secret: &str) -> Result<(), String> {
            Ok(())
        }
        fn delete(&self, _account: &str) -> Result<(), String> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct TestMediaServerCredentials;

    impl senzo_integrations::MediaServerCredentialStore for TestMediaServerCredentials {
        fn get(&self, _account: &str) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn set(&self, _account: &str, _secret: &str) -> Result<(), String> {
            Ok(())
        }
        fn delete(&self, _account: &str) -> Result<(), String> {
            Ok(())
        }
    }

    fn test_debrid_manager() -> DebridManager {
        DebridManager::new(Arc::new(TestDebridCredentials)).expect("test debrid manager")
    }

    fn test_media_server_manager() -> MediaServerManager {
        MediaServerManager::new(
            Arc::new(TestSettingsRepository),
            Arc::new(TestMediaServerCredentials),
        )
        .expect("test media-server manager")
    }

    fn test_source_aggregator() -> SourceAggregator {
        SourceAggregator::new(Vec::<Arc<dyn senzo_sources::SourceAdapter>>::new())
            .with_debrid(test_debrid_manager())
    }

    fn test_plugin_manager() -> PluginManager {
        PluginManager::new(
            Arc::new(TestSettingsRepository),
            Arc::new(TestPluginSettings),
            std::env::temp_dir().join("senzo-ui-plugin-test-cache"),
        )
    }

    impl SettingsRepository for TestSettingsRepository {
        fn get(&self, _key: &SettingKey) -> Result<Option<String>, RepositoryError> {
            Ok(None)
        }

        fn set(&self, _key: &SettingKey, _value: &str) -> Result<(), RepositoryError> {
            Ok(())
        }
    }

    #[test]
    fn named_mpv_chapters_expose_intro_recap_and_credits_skip_targets() {
        let mut snapshot = PlaybackSnapshot::default();
        snapshot.position_seconds = Some(12.0);
        snapshot.duration_seconds = Some(180.0);
        snapshot.chapters = vec![
            senzo_player::PlaybackChapter {
                index: 0,
                start_seconds: 0.0,
                title: Some("Intro".to_owned()),
            },
            senzo_player::PlaybackChapter {
                index: 1,
                start_seconds: 30.0,
                title: Some("Episode".to_owned()),
            },
        ];
        assert_eq!(
            chapter_skip_target(&snapshot),
            Some(("SKIP INTRO".to_owned(), 30.0))
        );

        snapshot.position_seconds = Some(145.0);
        snapshot.chapters = vec![
            senzo_player::PlaybackChapter {
                index: 0,
                start_seconds: 0.0,
                title: Some("Episode".to_owned()),
            },
            senzo_player::PlaybackChapter {
                index: 1,
                start_seconds: 140.0,
                title: Some("Credits".to_owned()),
            },
        ];
        assert_eq!(
            chapter_skip_target(&snapshot),
            Some(("SKIP CREDITS".to_owned(), 180.0))
        );

        snapshot.chapters[1].title = Some("Scene two".to_owned());
        assert_eq!(chapter_skip_target(&snapshot), None);
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
    fn home_card_focus_identity_is_stable_across_reordering_and_pagination() {
        let first = home_card_focus_id(HomeShelfKey::Trending, MediaType::Movie, 42);
        let after_reorder = home_card_focus_id(HomeShelfKey::Trending, MediaType::Movie, 42);
        let second = home_card_focus_id(HomeShelfKey::Trending, MediaType::Movie, 43);
        let tv_collision = home_card_focus_id(HomeShelfKey::Trending, MediaType::Tv, 42);

        assert_eq!(first, after_reorder);
        assert_ne!(first, second);
        assert_ne!(first, tv_collision);
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
    fn featured_sources_publish_in_saved_order_even_when_a_later_source_finishes_first() {
        let mut settings = HomeSettings::default();
        assert!(settings.set_hero_source_enabled("popular-movies", true));
        assert!(settings.move_section("popular-movies", 1));

        let mut pending = HashSet::from([HomeShelfKey::Trending, HomeShelfKey::PopularMovies]);
        assert!(ready_home_hero_sources(&settings, &pending).is_empty());

        pending.remove(&HomeShelfKey::PopularMovies);
        assert!(ready_home_hero_sources(&settings, &pending).is_empty());

        pending.remove(&HomeShelfKey::Trending);
        assert_eq!(
            ready_home_hero_sources(&settings, &pending),
            vec![HomeShelfKey::Trending, HomeShelfKey::PopularMovies]
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

    #[test]
    fn home_collection_artwork_requests_use_stable_keys_and_a_hard_cap() {
        let make_folder = |id: String, cover_artwork_path: Option<String>| HomeCollectionFolder {
            id,
            title: "Folder".to_owned(),
            poster_shape: HomeCollectionPosterShape::Poster,
            cover_emoji: None,
            cover_artwork_path,
            hide_title: false,
        };
        let collection = HomeCollection {
            id: "collection".to_owned(),
            title: "Collection".to_owned(),
            folders: vec![
                make_folder("cover".to_owned(), Some("/covers/one.jpg".to_owned())),
                make_folder("emoji-only".to_owned(), None),
                make_folder("second".to_owned(), Some("/covers/two.jpg".to_owned())),
            ],
        };

        let requests = home_collection_artwork_requests(std::slice::from_ref(&collection));
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0].0,
            home_collection_folder_key("collection", "cover").unwrap()
        );
        assert_eq!(requests[0].1, "/covers/one.jpg");
        assert_eq!(
            requests[1].0,
            home_collection_folder_key("collection", "second").unwrap()
        );

        let first_collection = HomeCollection {
            id: "first".to_owned(),
            title: "First collection".to_owned(),
            folders: (0..100)
                .map(|index| {
                    make_folder(
                        format!("first-{index}"),
                        Some(format!("/covers/first-{index}.jpg")),
                    )
                })
                .collect(),
        };
        let second_collection = HomeCollection {
            id: "second".to_owned(),
            title: "Second collection".to_owned(),
            folders: (100..(MAX_HOME_COLLECTION_ARTWORK_ITEMS + 10))
                .map(|index| {
                    make_folder(
                        format!("second-{index}"),
                        Some(format!("/covers/second-{index}.jpg")),
                    )
                })
                .collect(),
        };
        assert_eq!(
            home_collection_artwork_requests(&[first_collection, second_collection]).len(),
            MAX_HOME_COLLECTION_ARTWORK_ITEMS
        );
    }

    #[gpui::test]
    async fn keyboard_navigation_back_forward_and_focus_restoration(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);

        // GPUI CE's test scheduler is deliberately single-threaded. Enter a
        // current-thread Tokio runtime to provide reactor context to Tokio
        // APIs without introducing worker threads that would cross the GPUI
        // test scheduler's thread boundary.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test Tokio runtime should initialize");
        let _runtime_guard = runtime.enter();

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
                ShellServices {
                    tokio: runtime.handle().clone(),
                    metadata,
                    artwork,
                    settings: Arc::new(TestSettingsRepository),
                    addon_manager: test_addon_manager(),
                    plugin_manager: test_plugin_manager(),
                    debrid_manager: test_debrid_manager(),
                    media_server_manager: test_media_server_manager(),
                    source_aggregator: test_source_aggregator(),
                },
                true,
                window,
                cx,
            );
            // Keep navigation traversal deterministic: visiting Movies and TV should not
            // start real discovery requests from the single-threaded GPUI test scheduler.
            shell.home_catalogue_status = HomeCatalogueStatus::Loaded;
            shell.movie_discovery.status = DiscoveryStatus::Empty;
            shell.tv_discovery.status = DiscoveryStatus::Empty;
            let home_focus = shell
                .navigation_focus
                .get(&AppPage::Home)
                .expect("home focus handle exists")
                .clone();
            window.focus(&home_focus, cx);
            shell
        });

        window_cx.update(|_, app| bind_shell_keys(app));

        let sidebar = window_cx
            .debug_bounds("senzo-sidebar")
            .expect("sidebar should be present in the rendered shell");
        let header = window_cx
            .debug_bounds("senzo-header")
            .expect("header should be present in the rendered shell");
        let page = window_cx
            .debug_bounds("page-content")
            .expect("main page viewport should be present in the rendered shell");
        let hero = window_cx
            .debug_bounds("home-featured-hero")
            .expect("Home featured hero should be present in the rendered shell");
        let shelf = window_cx
            .debug_bounds("home-trending-shelf-scroll")
            .expect("built-in Home shelf rail should be present in the rendered shell");
        let refresh = window_cx
            .debug_bounds("home-refresh-catalogues")
            .expect("Home catalogue refresh control should be present in the rendered shell");

        let sidebar_x = f32::from(sidebar.origin.x);
        let sidebar_width = f32::from(sidebar.size.width);
        let header_y = f32::from(header.origin.y);
        let header_height = f32::from(header.size.height);
        let page_x = f32::from(page.origin.x);
        let page_width = f32::from(page.size.width);
        let page_height = f32::from(page.size.height);
        let hero_x = f32::from(hero.origin.x);
        let hero_y = f32::from(hero.origin.y);
        let hero_width = f32::from(hero.size.width);
        let hero_height = f32::from(hero.size.height);
        let shelf_width = f32::from(shelf.size.width);
        let refresh_x = f32::from(refresh.origin.x);
        let refresh_width = f32::from(refresh.size.width);

        assert!(
            sidebar_x.abs() < 1.0,
            "sidebar should anchor to the left edge"
        );
        assert!(
            (sidebar_width - Theme::SIDEBAR_WIDTH).abs() < 1.0,
            "sidebar width should match the native design token"
        );
        assert!(
            (header_height - Theme::HEADER_HEIGHT).abs() < 1.0,
            "header height should match the native design token"
        );
        assert!(
            (page_x - (sidebar_x + sidebar_width)).abs() < 1.0,
            "main viewport should begin immediately after the persistent sidebar"
        );
        assert!(page_width > 0.0 && page_height > 0.0);
        assert!(
            (hero_height - 286.0).abs() < 1.0,
            "featured hero should retain its 286px cinematic height"
        );
        assert!(
            hero_width > 0.0 && hero_width < page_width && hero_x >= page_x,
            "hero should fit inside the padded main viewport"
        );
        assert!(
            hero_y >= header_y + header_height,
            "Home hero must start below the persistent header"
        );
        assert!(
            shelf_width > 0.0 && shelf_width <= page_width,
            "horizontal shelf rail must remain within the main viewport"
        );
        assert!(
            refresh_x >= page_x && refresh_x + refresh_width <= page_x + page_width,
            "Home toolbar control must remain within the main viewport"
        );

        // Structural visual-regression fixtures at the supported minimum,
        // standard 1080p, and full-HD desktop viewport sizes. GPUI CE 0.2.2's
        // test context exposes layout bounds and input simulation rather than
        // pixel readback, so assert the real rendered geometry here.
        for (width, height) in [(1024.0, 680.0), (1280.0, 720.0), (1920.0, 1080.0)] {
            window_cx.simulate_resize(size(px(width), px(height)));
            // The test platform reports the new native viewport immediately;
            // drain GPUI's scheduled frame before inspecting its painted bounds.
            window_cx.update_entity(&shell, |_, cx| cx.notify());
            window_cx.update(|window, _| window.refresh());
            window_cx.run_until_parked();
            let viewport = window_cx.update(|window, _| window.viewport_size());
            assert!(
                (f32::from(viewport.width) - width).abs() < 1.0
                    && (f32::from(viewport.height) - height).abs() < 1.0,
                "GPUI test window should adopt the requested {width}x{height} viewport"
            );

            let resized_sidebar = window_cx
                .debug_bounds("senzo-sidebar")
                .expect("sidebar should remain rendered after resizing");
            let resized_header = window_cx
                .debug_bounds("senzo-header")
                .expect("header should remain rendered after resizing");
            let resized_page = window_cx
                .debug_bounds("page-content")
                .expect("page viewport should remain rendered after resizing");
            let resized_hero = window_cx
                .debug_bounds("home-featured-hero")
                .expect("Home hero should remain rendered after resizing");
            let resized_refresh = window_cx
                .debug_bounds("home-refresh-catalogues")
                .expect("Home refresh control should remain rendered after resizing");

            let sidebar_right = f32::from(resized_sidebar.origin.x + resized_sidebar.size.width);
            let page_x = f32::from(resized_page.origin.x);
            let page_right = page_x + f32::from(resized_page.size.width);
            let hero_x = f32::from(resized_hero.origin.x);
            let hero_right = hero_x + f32::from(resized_hero.size.width);
            let refresh_x = f32::from(resized_refresh.origin.x);
            let refresh_right = refresh_x + f32::from(resized_refresh.size.width);

            assert!(
                (f32::from(resized_sidebar.size.width) - Theme::SIDEBAR_WIDTH).abs() < 1.0,
                "sidebar token should remain stable at {width}x{height}"
            );
            assert!(
                (f32::from(resized_header.size.height) - Theme::HEADER_HEIGHT).abs() < 1.0,
                "header token should remain stable at {width}x{height}"
            );
            assert!(
                (page_x - sidebar_right).abs() < 1.0,
                "page should begin directly after the sidebar at {width}x{height}"
            );
            assert!(
                page_right <= width + 1.0 && f32::from(resized_page.size.height) > 0.0,
                "page viewport should remain within the window at {width}x{height} (page_x={page_x}, page_right={page_right}, page_width={}, viewport_width={})",
                f32::from(resized_page.size.width),
                f32::from(viewport.width)
            );
            assert!(
                hero_x >= page_x && hero_right <= page_right + 1.0,
                "featured hero should remain inside the page viewport at {width}x{height}"
            );
            assert!(
                refresh_x >= page_x && refresh_right <= page_right + 1.0,
                "Home refresh control should remain inside the page viewport at {width}x{height}"
            );
        }
        window_cx.simulate_resize(size(px(1280.0), px(720.0)));

        // Verify both documented keyboard paths enter and leave fullscreen.
        let initial_fullscreen = window_cx.update(|window, _| window.is_fullscreen());
        window_cx.simulate_keystrokes("f11");
        assert_ne!(
            window_cx.update(|window, _| window.is_fullscreen()),
            initial_fullscreen,
            "F11 should toggle fullscreen"
        );
        window_cx.simulate_keystrokes("alt-enter");
        assert_eq!(
            window_cx.update(|window, _| window.is_fullscreen()),
            initial_fullscreen,
            "Alt+Enter should restore the previous window mode"
        );

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

        // Exercise pointer sidebar navigation and platform mouse Navigate
        // events used by physical Back/Forward side buttons.
        let movies_bounds = window_cx
            .debug_bounds("navigation-movies")
            .expect("Movies navigation item should be present in the rendered window");
        let movies_center = point(
            movies_bounds.origin.x + movies_bounds.size.width / 2.0,
            movies_bounds.origin.y + movies_bounds.size.height / 2.0,
        );
        window_cx.simulate_click(movies_center, gpui::Modifiers::default());
        assert_eq!(
            shell.read_with(window_cx, |shell, _| shell.navigation.current()),
            AppPage::Movies
        );

        window_cx.simulate_mouse_down(
            movies_center,
            MouseButton::Navigate(NavigationDirection::Back),
            gpui::Modifiers::default(),
        );
        assert_eq!(
            shell.read_with(window_cx, |shell, _| shell.navigation.current()),
            AppPage::Settings
        );
        window_cx.simulate_mouse_up(
            movies_center,
            MouseButton::Navigate(NavigationDirection::Back),
            gpui::Modifiers::default(),
        );

        window_cx.simulate_mouse_down(
            movies_center,
            MouseButton::Navigate(NavigationDirection::Forward),
            gpui::Modifiers::default(),
        );
        assert_eq!(
            shell.read_with(window_cx, |shell, _| shell.navigation.current()),
            AppPage::Movies
        );
        window_cx.simulate_mouse_up(
            movies_center,
            MouseButton::Navigate(NavigationDirection::Forward),
            gpui::Modifiers::default(),
        );
    }
}
