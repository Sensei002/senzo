use std::{env, sync::Arc, time::{Duration, Instant}};

use anyhow::{Context, ensure};
use senzo_addons::{AddonManager, AddonSecretStore};
use senzo_artwork::{ArtworkConfig, ArtworkService, ArtworkSize, ArtworkUrlResolver};
use senzo_debrid::{DebridCredentialStore, DebridManager};
use senzo_domain::{ProfileRepository, SettingsRepository};
use senzo_integrations::{MediaServerCredentialStore, MediaServerManager};
use senzo_metadata::{
    DEFAULT_API_BASE_URL, DiscoverRequest, DiscoverSort, MediaType, MetadataClient,
    MetadataClientConfig, SearchType,
};
use senzo_network::{HttpClient, HttpPolicy};
use senzo_platform::{AppDirectories, CredentialStore, KeyringCredentialStore};
use senzo_player::{MpvConfig, MpvPlayer, PlaybackStatus, TrackKind};
use senzo_plugins::{PluginManager, PluginSettingsStore};
use senzo_sources::{
    AddonSourceAdapter, MediaServerSourceAdapter, PluginSourceAdapter, SourceAdapter,
    SourceAggregator,
};
use senzo_storage::SqliteDatabase;
use tokio::runtime::{Builder, Runtime};
use tokio_util::sync::CancellationToken;

struct PlatformAddonSecrets(Arc<dyn CredentialStore>);

impl AddonSecretStore for PlatformAddonSecrets {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        self.0.get(key).map_err(|error| error.to_string())
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        self.0.set(key, secret).map_err(|error| error.to_string())
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        self.0.delete(key).map_err(|error| error.to_string())
    }
}

struct PlatformPluginSettings(Arc<dyn CredentialStore>);

impl PluginSettingsStore for PlatformPluginSettings {
    fn get(&self, account: &str) -> Result<Option<String>, String> {
        self.0.get(account).map_err(|error| error.to_string())
    }

    fn set(&self, account: &str, value: &str) -> Result<(), String> {
        self.0
            .set(account, value)
            .map_err(|error| error.to_string())
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        self.0.delete(account).map_err(|error| error.to_string())
    }
}

struct PlatformDebridSecrets(Arc<dyn CredentialStore>);

impl DebridCredentialStore for PlatformDebridSecrets {
    fn get(&self, account: &str) -> Result<Option<String>, String> {
        self.0.get(account).map_err(|error| error.to_string())
    }
    fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        self.0
            .set(account, secret)
            .map_err(|error| error.to_string())
    }
    fn delete(&self, account: &str) -> Result<(), String> {
        self.0.delete(account).map_err(|error| error.to_string())
    }
}

struct PlatformMediaServerSecrets(Arc<dyn CredentialStore>);

impl MediaServerCredentialStore for PlatformMediaServerSecrets {
    fn get(&self, account: &str) -> Result<Option<String>, String> {
        self.0.get(account).map_err(|error| error.to_string())
    }
    fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        self.0
            .set(account, secret)
            .map_err(|error| error.to_string())
    }
    fn delete(&self, account: &str) -> Result<(), String> {
        self.0.delete(account).map_err(|error| error.to_string())
    }
}

struct AppServices {
    _settings: Arc<dyn SettingsRepository>,
    _profiles: Arc<dyn ProfileRepository>,
    _metadata: MetadataClient,
    _artwork: ArtworkService,
    _credentials: Arc<dyn CredentialStore>,
    _addon_manager: AddonManager,
    _plugin_manager: PluginManager,
    _debrid_manager: DebridManager,
    _media_server_manager: MediaServerManager,
    _source_aggregator: SourceAggregator,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("SENZO startup failed: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    senzo_diagnostics::init()
        .map_err(|error| anyhow::anyhow!("initialize structured diagnostics: {error}"))?;
    let directories =
        AppDirectories::discover().context("resolve native application directories")?;
    let database = Arc::new(
        SqliteDatabase::open(directories.database_file())
            .context("open the local SENZO database and apply migrations")?,
    );
    let http =
        HttpClient::new(HttpPolicy::default()).context("initialize the shared HTTP client")?;
    let api_base_url =
        env::var("SENZO_API_BASE_URL").unwrap_or_else(|_| DEFAULT_API_BASE_URL.to_owned());
    let metadata = MetadataClient::new(
        http.clone(),
        MetadataClientConfig {
            base_url: api_base_url.clone(),
            ..Default::default()
        },
    )
    .context("configure the hosted SENZO metadata API client")?;
    let artwork = ArtworkService::new(
        http,
        directories.artwork_cache_dir(),
        ArtworkConfig::default(),
    )
    .context("initialize the bounded native artwork cache")?;

    let credentials: Arc<dyn CredentialStore> =
        Arc::new(KeyringCredentialStore::new("SENZO Desktop"));
    let addon_manager = AddonManager::new(
        database.clone(),
        Arc::new(PlatformAddonSecrets(credentials.clone())),
    );
    let plugin_manager = PluginManager::new(
        database.clone(),
        Arc::new(PlatformPluginSettings(credentials.clone())),
        directories.data_dir().join("plugins"),
    );

    let debrid_manager = DebridManager::new(Arc::new(PlatformDebridSecrets(credentials.clone())))
        .context("initialize the optional debrid provider manager")?;
    let media_server_manager = MediaServerManager::new(
        database.clone(),
        Arc::new(PlatformMediaServerSecrets(credentials.clone())),
    )
    .context("initialize the Jellyfin and Emby provider manager")?;
    let source_adapters: Vec<Arc<dyn SourceAdapter>> = vec![
        Arc::new(AddonSourceAdapter::new(addon_manager.clone(), "default")),
        Arc::new(PluginSourceAdapter::new(plugin_manager.clone(), "default")),
        Arc::new(MediaServerSourceAdapter::new(
            media_server_manager.clone(),
            "default",
        )),
    ];
    let source_aggregator =
        SourceAggregator::new(source_adapters).with_debrid(debrid_manager.clone());

    let services = AppServices {
        _settings: database.clone(),
        _profiles: database.clone(),
        _metadata: metadata.clone(),
        _artwork: artwork.clone(),
        _credentials: credentials,
        _addon_manager: addon_manager.clone(),
        _plugin_manager: plugin_manager.clone(),
        _debrid_manager: debrid_manager.clone(),
        _media_server_manager: media_server_manager.clone(),
        _source_aggregator: source_aggregator.clone(),
    };

    let schema_version = database
        .schema_version()
        .context("read database schema version")?;
    tracing::info!(
        schema_version,
        data_directory = %directories.data_dir().display(),
        api_base = %api_base_url,
        "SENZO foundation initialized"
    );

    let runtime = create_runtime().context("create application-owned Tokio runtime")?;
    if env::args().any(|argument| argument == "--metadata-smoke-test") {
        runtime
            .block_on(run_metadata_smoke_test(&metadata, &artwork))
            .context("validate hosted metadata and native artwork integration")?;
        tracing::info!("hosted metadata and artwork integration smoke test passed");
        drop(services);
        drop(runtime);
        return Ok(());
    }

    let args = env::args().collect::<Vec<_>>();
    if let Some(index) = args.iter().position(|argument| argument == "--mpv-smoke-test") {
        let url = args.get(index + 1).context(
            "usage: senzo --mpv-smoke-test <direct-http-or-https-media-url>"
        )?;
        runtime
            .block_on(run_mpv_smoke_test(url))
            .context("validate native MPV playback, controls and teardown")?;
        drop(services);
        drop(runtime);
        return Ok(());
    }

    let smoke_test = env::args().any(|argument| argument == "--smoke-test");
    senzo_ui::run(
        senzo_ui::ShellServices {
            tokio: runtime.handle().clone(),
            metadata,
            artwork,
            settings: database.clone(),
            addon_manager,
            plugin_manager,
            debrid_manager,
            media_server_manager,
            source_aggregator,
        },
        smoke_test,
    );

    drop(services);
    drop(runtime);
    Ok(())
}

async fn run_mpv_smoke_test(url:&str)->anyhow::Result<()> {
    let player=MpvPlayer::spawn(MpvConfig::default())
        .context("load the native libmpv runtime; install it or set SENZO_LIBMPV_PATH")?;
    player.load(url,&std::collections::BTreeMap::new())
        .context("load the direct HTTP(S) test source")?;

    let snapshot=wait_for_mpv_video(&player,Duration::from_secs(35)).await?;
    ensure!(
        snapshot.audio_codec.is_some()
            || snapshot.tracks.iter().any(|track|track.kind==TrackKind::Audio),
        "the test source did not expose an audio track"
    );
    println!(
        "PASS  native MPV video stream started: {}x{}; video codec: {}; audio codec: {}",
        snapshot.video_width.unwrap_or_default(),
        snapshot.video_height.unwrap_or_default(),
        snapshot.video_codec.as_deref().unwrap_or("unknown"),
        snapshot.audio_codec.as_deref().unwrap_or("not detected")
    );
    println!(
        "Tracks: {} audio, {} subtitle",
        snapshot.tracks.iter().filter(|track|track.kind==TrackKind::Audio).count(),
        snapshot.tracks.iter().filter(|track|track.kind==TrackKind::Subtitle).count()
    );

    player.pause().context("pause the native playback session")?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    ensure!(player.snapshot().paused,"MPV did not report the paused state");
    player.seek_relative(5.0).context("seek the active source")?;
    player.play().context("resume the native playback session")?;
    player.set_volume(35.0).context("set playback volume")?;
    player.set_muted(true).context("mute playback")?;
    player.set_muted(false).context("unmute playback")?;
    player.set_speed(1.25).context("change playback speed")?;
    player.set_speed(1.0).context("restore normal playback speed")?;

    // Replace the currently playing source to exercise MPV's single-session
    // replace path, then stop and start it again to exercise repeated sessions.
    player.load(url,&std::collections::BTreeMap::new())
        .context("replace the active source in the existing MPV session")?;
    let _=wait_for_mpv_video(&player,Duration::from_secs(35)).await?;
    tokio::time::sleep(Duration::from_secs(2)).await;

    let stop=tokio::task::spawn_blocking({
        let player=player.clone();
        move||player.stop(Duration::from_secs(8))
    }).await.context("join the first MPV stop acknowledgement task")?;
    stop.context("wait for the first MPV stop acknowledgement")?;

    player.load(url,&std::collections::BTreeMap::new())
        .context("start a second playback session on the existing MPV owner")?;
    let _=wait_for_mpv_video(&player,Duration::from_secs(35)).await?;

    let shutdown=tokio::task::spawn_blocking(move||{
        player.stop(Duration::from_secs(8)).context("wait for second MPV stop acknowledgement")?;
        player.shutdown().context("shut down the MPV owner thread")?;
        Ok::<(),anyhow::Error>(())
    }).await.context("join the final MPV teardown task")?;
    shutdown?;
    println!("PASS  pause, seek, resume, volume, mute, speed, source replacement, repeated session, stop acknowledgements and owner-thread shutdown");
    println!("VISUAL/AUDIO CHECK: confirm the native video window was visible and sound was audible during this test.");
    Ok(())
}

async fn wait_for_mpv_video(
    player:&MpvPlayer,
    timeout:Duration,
)->anyhow::Result<senzo_player::PlaybackSnapshot>{
    let deadline=Instant::now()+timeout;
    loop {
        let current=player.snapshot();
        if current.status==PlaybackStatus::Error {
            anyhow::bail!("libmpv entered an error state: {:?}",current.diagnostics.last());
        }
        if current.status==PlaybackStatus::Playing
            && current.video_width.is_some_and(|width|width>0)
            && current.video_height.is_some_and(|height|height>0)
        {
            return Ok(current);
        }
        if Instant::now()>=deadline {
            anyhow::bail!("MPV did not start a video stream with nonzero dimensions within {:?}; last state: {:?}",timeout,current.status);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn run_metadata_smoke_test(
    metadata: &MetadataClient,
    artwork: &ArtworkService,
) -> anyhow::Result<()> {
    let cancellation = CancellationToken::new();

    let health = metadata.health(&cancellation).await?;
    ensure!(
        health.service == "SENZO API" && health.status == "ok" && health.tmdb_configured,
        "hosted SENZO API health is invalid or the selected Vercel environment is missing its server-side TMDB_READ_ACCESS_TOKEN; configure it in the Vercel Preview environment, separately from the GitHub Actions bypass secret"
    );
    println!("PASS  hosted API health and server-side TMDB configuration");

    let trending = metadata.trending(1, "en-US", &cancellation).await?;
    ensure!(!trending.results.is_empty(), "trending catalogue is empty");
    let popular_movies = metadata.popular_movies(1, "en-US", &cancellation).await?;
    ensure!(
        !popular_movies.results.is_empty(),
        "popular movies catalogue is empty"
    );
    let popular_tv = metadata.popular_tv(1, "en-US", &cancellation).await?;
    ensure!(
        !popular_tv.results.is_empty(),
        "popular TV catalogue is empty"
    );
    println!("PASS  all three Home catalogues");

    let movie_genres = metadata
        .genres(MediaType::Movie, "en-US", &cancellation)
        .await?;
    ensure!(
        movie_genres.genres.iter().any(|genre| genre.id == 28),
        "movie genre catalogue is missing Action"
    );
    let tv_genres = metadata
        .genres(MediaType::Tv, "en-US", &cancellation)
        .await?;
    ensure!(
        tv_genres.genres.iter().any(|genre| genre.id == 18),
        "TV genre catalogue is missing Drama"
    );
    let movie_discovery = metadata
        .discover(
            DiscoverRequest {
                media_type: MediaType::Movie,
                genre_id: Some(28),
                year: None,
                sort: DiscoverSort::Popularity,
                page: 1,
                language: "en-US",
            },
            &cancellation,
        )
        .await?;
    ensure!(
        !movie_discovery.results.is_empty(),
        "filtered movie discovery returned no titles"
    );
    let tv_discovery = metadata
        .discover(
            DiscoverRequest {
                media_type: MediaType::Tv,
                genre_id: Some(18),
                year: None,
                sort: DiscoverSort::Popularity,
                page: 1,
                language: "en-US",
            },
            &cancellation,
        )
        .await?;
    ensure!(
        !tv_discovery.results.is_empty(),
        "filtered TV discovery returned no titles"
    );
    println!("PASS  movie/TV genres and filtered discovery");

    let multi_search = metadata
        .search("Inception", SearchType::Multi, 1, "en-US", &cancellation)
        .await?;
    ensure!(
        multi_search.results.iter().any(|item| item.id == 27_205),
        "multi-search did not contain Inception"
    );
    let movie_search = metadata
        .search("Inception", SearchType::Movie, 1, "en-US", &cancellation)
        .await?;
    ensure!(
        movie_search.results.iter().any(|item| item.id == 27_205),
        "movie-only search did not contain Inception"
    );
    let tv_search = metadata
        .search("Game of Thrones", SearchType::Tv, 1, "en-US", &cancellation)
        .await?;
    ensure!(
        tv_search.results.iter().any(|item| item.id == 1_399),
        "TV-only search did not contain Game of Thrones"
    );
    println!("PASS  multi, movie-only and TV-only Search");

    let movie = metadata.movie(27_205, &cancellation).await?;
    ensure!(movie.id == 27_205, "movie Details returned the wrong ID");
    ensure!(
        movie.credits.is_some() && movie.videos.is_some() && movie.images.is_some(),
        "movie Details is missing credits, videos or image metadata"
    );
    ensure!(
        movie.recommendations.is_some() || movie.similar.is_some(),
        "movie Details is missing related-title metadata"
    );
    ensure!(
        movie
            .external_ids
            .as_ref()
            .is_some_and(|ids| ids.imdb_id.is_some()),
        "movie Details is missing external IDs"
    );
    let movie_external_ids = metadata.movie_external_ids(27_205, &cancellation).await?;
    ensure!(
        movie_external_ids.imdb_id.is_some(),
        "dedicated movie external-ID route returned no IMDb ID"
    );
    println!("PASS  movie Details, credits, videos, images and external IDs");

    let tv = metadata.tv(1_399, &cancellation).await?;
    ensure!(tv.id == 1_399, "TV Details returned the wrong ID");
    ensure!(
        !tv.seasons.is_empty() && tv.aggregate_credits.is_some(),
        "TV Details is missing season or aggregate credit metadata"
    );
    ensure!(
        tv.videos.is_some() && tv.images.is_some() && tv.external_ids.is_some(),
        "TV Details is missing video, image or external-ID metadata"
    );
    ensure!(
        tv.recommendations.is_some() || tv.similar.is_some(),
        "TV Details is missing related-title metadata"
    );
    let tv_external_ids = metadata.tv_external_ids(1_399, &cancellation).await?;
    ensure!(
        tv_external_ids.imdb_id.is_some(),
        "dedicated TV external-ID route returned no IMDb ID"
    );
    let season = metadata.season(1_399, 1, &cancellation).await?;
    ensure!(
        season.season_number == 1 && !season.episodes.is_empty(),
        "TV season 1 is missing episode metadata"
    );
    let episode = metadata.episode(1_399, 1, 1, &cancellation).await?;
    ensure!(
        episode.id > 0 && !episode.name.trim().is_empty(),
        "TV episode details are incomplete"
    );
    println!("PASS  TV Details, credits, videos, images, season and episode");

    let person = metadata.person(287, "en-US", &cancellation).await?;
    ensure!(
        person.id == 287 && person.name == "Brad Pitt",
        "Person Details returned an unexpected person"
    );
    ensure!(
        person.profile_path.is_some() && person.combined_credits.is_some(),
        "Person Details is missing profile artwork or combined credits"
    );
    println!("PASS  Person Details and combined filmography");

    let poster_path = movie
        .poster_path
        .as_deref()
        .context("movie Details returned no poster path for artwork validation")?;
    let poster_url = ArtworkUrlResolver.resolve_tmdb_path(poster_path, ArtworkSize::W342)?;
    let poster = artwork.load(&poster_url, &cancellation).await?;
    ensure!(
        poster.width() > 0
            && poster.height() > 0
            && poster.rgba8().len()
                == (poster.width() as usize)
                    .saturating_mul(poster.height() as usize)
                    .saturating_mul(4),
        "native artwork decoder returned invalid RGBA8 pixels"
    );
    println!(
        "PASS  native TMDB artwork download and decode ({} × {}, {:?})",
        poster.width(),
        poster.height(),
        poster.format()
    );

    Ok(())
}

fn create_runtime() -> std::io::Result<Runtime> {
    Builder::new_multi_thread()
        .enable_all()
        .thread_name("senzo-tokio")
        .worker_threads(2)
        .build()
}
