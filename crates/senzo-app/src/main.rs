use std::{env, sync::Arc};

use anyhow::Context;
use senzo_domain::{ProfileRepository, SettingsRepository};
use senzo_metadata::{DEFAULT_API_BASE_URL, MetadataClient, MetadataClientConfig};
use senzo_network::{HttpClient, HttpPolicy};
use senzo_platform::{AppDirectories, CredentialStore, KeyringCredentialStore};
use senzo_storage::SqliteDatabase;
use tokio::runtime::{Builder, Runtime};

struct AppServices {
    _settings: Arc<dyn SettingsRepository>,
    _profiles: Arc<dyn ProfileRepository>,
    _metadata: MetadataClient,
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
    let http = HttpClient::new(HttpPolicy::default())
        .context("initialize the shared HTTP client")?;
    let api_base_url =
        env::var("SENZO_API_BASE_URL").unwrap_or_else(|_| DEFAULT_API_BASE_URL.to_owned());
    let metadata = MetadataClient::new(
        http,
        MetadataClientConfig {
            base_url: api_base_url,
            ..Default::default()
        },
    )
    .context("configure the hosted SENZO metadata API client")?;

    let services = AppServices {
        _settings: database.clone(),
        _profiles: database.clone(),
        _metadata: metadata,
        _credentials: Arc::new(KeyringCredentialStore::new("SENZO Desktop")),
    };

    let schema_version = database
        .schema_version()
        .context("read database schema version")?;
    tracing::info!(
        schema_version,
        data_directory = %directories.data_dir().display(),
        api_base = %DEFAULT_API_BASE_URL,
        "SENZO foundation initialized"
    );

    let runtime = create_runtime().context("create application-owned Tokio runtime")?;
    let smoke_test = env::args().any(|argument| argument == "--smoke-test");
    senzo_ui::run(runtime.handle().clone(), smoke_test);

    drop(services);
    drop(runtime);
    Ok(())
}

fn create_runtime() -> std::io::Result<Runtime> {
    Builder::new_multi_thread()
        .enable_all()
        .thread_name("senzo-tokio")
        .worker_threads(2)
        .build()
}
