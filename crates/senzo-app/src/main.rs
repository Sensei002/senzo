use std::{env, sync::Arc};

use anyhow::{Context, ensure};
use senzo_artwork::{
    ArtworkConfig, ArtworkService, ArtworkSize, ArtworkUrlResolver,
};
use senzo_domain::{ProfileRepository, SettingsRepository};
use senzo_metadata::{
    DEFAULT_API_BASE_URL, MetadataClient, MetadataClientConfig, SearchType,
};
use senzo_network::{HttpClient, HttpPolicy};
use senzo_platform::{AppDirectories, CredentialStore, KeyringCredentialStore};
use senzo_storage::SqliteDatabase;
use tokio::runtime::{Builder, Runtime};
use tokio_util::sync::CancellationToken;

struct AppServices {
    _settings: Arc<dyn SettingsRepository>,
    _profiles: Arc<dyn ProfileRepository>,
    _metadata: MetadataClient,
    _artwork: ArtworkService,
    _credentials: Arc<dyn CredentialStore>,
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

    let services = AppServices {
        _settings: database.clone(),
        _profiles: database.clone(),
        _metadata: metadata.clone(),
        _artwork: artwork.clone(),
        _credentials: Arc::new(KeyringCredentialStore::new("SENZO Desktop")),
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

    let smoke_test = env::args().any(|argument| argument == "--smoke-test");
    senzo_ui::run(runtime.handle().clone(), smoke_test, metadata, artwork);

    drop(services);
    drop(runtime);
    Ok(())
}

async fn run_metadata_smoke_test(
    metadata: &MetadataClient,
    artwork: &ArtworkService,
) -> anyhow::Result<()> {
    let cancellation = CancellationToken::new();

    let health = metadata.health(&cancellation).await?;
    ensure!(
        health.service == "SENZO API" && health.status == "ok" && health.tmdb_configured,
        "hosted SENZO API health or server-side TMDB configuration is invalid"
    );
    println!("PASS  hosted API health and server-side TMDB configuration");

    let trending = metadata.trending(1, "en-US", &cancellation).await?;
    ensure!(!trending.results.is_empty(), "trending catalogue is empty");
    let popular_movies = metadata
        .popular_movies(1, "en-US", &cancellation)
        .await?;
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
        .search(
            "Game of Thrones",
            SearchType::Tv,
            1,
            "en-US",
            &cancellation,
        )
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
        movie.external_ids.as_ref().is_some_and(|ids| ids.imdb_id.is_some()),
        "movie Details is missing external IDs"
    );
    let movie_external_ids = metadata
        .movie_external_ids(27_205, &cancellation)
        .await?;
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
