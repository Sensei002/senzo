use crate::models::pagination::{PaginationIdentity, PaginationScope, gate as pagination_gate};
use crate::models::{Fingerprint, SyncFingerprint, sync_fingerprint_changed};
use crate::{
    AddonItem, AppModel, AppModelField, MainWindow, NavigationController, NavigationIntent,
};
use core_env::DesktopEnv;
use slint::ComponentHandle;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex, OnceLock},
};
use stremio_core::{
    constants::PROFILE_STORAGE_KEY,
    models::{
        addon_details::{AddonDetails, Selected as AddonDetailsSelected},
        catalog_with_filters::CatalogWithFilters,
        common::Loadable,
    },
    runtime::{
        Env, Runtime, RuntimeAction,
        msg::{Action, ActionCatalogWithFilters, ActionCtx, ActionLoad},
    },
    types::{
        addon::Descriptor,
        api::{APIRequest, APIResult, SuccessResponse, fetch_api},
    },
};
use url::Url;

static LAST_SYNC_STATE: OnceLock<Mutex<Option<SyncFingerprint>>> = OnceLock::new();
static PENDING_ADDON_ORDER: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
static ADDON_REORDER_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
const ADDON_ORDER_STORAGE_KEY: &str = "senzo-addon-order";
const LEGACY_ADDON_ORDER_STORAGE_KEY: &str = "stremio-native-addon-order";

fn parse_manifest_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value.trim())
        .map_err(|error| format!("Invalid manifest URL: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("Add-on manifest URLs must use HTTP or HTTPS.".to_owned());
    }
    Ok(url)
}

fn publish_install_feedback(
    ui_weak: slint::Weak<MainWindow>,
    message: String,
    is_error: bool,
) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_loading(false);
            ui.set_addons_install_feedback(message.into());
            ui.set_addons_install_feedback_error(is_error);
        }
    });
}

fn move_item<T>(items: &mut Vec<T>, from: i32, to: i32) -> bool {
    let (Ok(from), Ok(to)) = (usize::try_from(from), usize::try_from(to)) else {
        return false;
    };
    if from == to || from >= items.len() || to >= items.len() {
        return false;
    }
    let item = items.remove(from);
    items.insert(to, item);
    true
}

fn apply_transport_order(addons: &mut [Descriptor], order: &[String]) -> bool {
    if addons.len() != order.len() {
        return false;
    }
    let ranks = order
        .iter()
        .enumerate()
        .map(|(index, transport_url)| (transport_url.as_str(), index))
        .collect::<std::collections::HashMap<_, _>>();
    if addons
        .iter()
        .any(|addon| !ranks.contains_key(addon.transport_url.as_str()))
    {
        return false;
    }
    addons.sort_by_key(|addon| ranks[addon.transport_url.as_str()]);
    true
}

fn ordered_installed(addons: &[Descriptor]) -> Vec<Descriptor> {
    let mut ordered = addons.to_vec();
    let pending = PENDING_ADDON_ORDER
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    apply_transport_order(&mut ordered, &pending);
    ordered
}

fn hash_addon(fingerprint: &mut Fingerprint, descriptor: &Descriptor, installed: bool) {
    fingerprint.bool(installed);
    fingerprint.str(&descriptor.manifest.id);
    fingerprint.str(&descriptor.manifest.name);
    fingerprint.u64(descriptor.manifest.version.major);
    fingerprint.u64(descriptor.manifest.version.minor);
    fingerprint.u64(descriptor.manifest.version.patch);
    fingerprint.str(descriptor.manifest.version.pre.as_str());
    fingerprint.str(descriptor.manifest.version.build.as_str());
    fingerprint.optional_str(descriptor.manifest.description.as_deref());
    fingerprint.optional_str(descriptor.manifest.logo.as_ref().map(Url::as_str));
    fingerprint.optional_str(descriptor.manifest.background.as_ref().map(Url::as_str));
    fingerprint.str(descriptor.transport_url.as_str());
    for addon_type in &descriptor.manifest.types {
        fingerprint.str(addon_type);
    }
    fingerprint.bool(descriptor.manifest.behavior_hints.configurable);
    fingerprint.bool(descriptor.manifest.behavior_hints.configuration_required);
    fingerprint.bool(descriptor.manifest.behavior_hints.adult);
    fingerprint.bool(descriptor.manifest.behavior_hints.p2p);
}

pub(crate) fn addon_identity(value: &str) -> String {
    let Ok(mut url) = Url::parse(value) else {
        return value.trim().trim_end_matches('/').to_ascii_lowercase();
    };
    url.set_query(None);
    url.set_fragment(None);
    url.as_str().trim_end_matches('/').to_ascii_lowercase()
}

/// Community directory entries use the base manifest URL, while a configured
/// add-on (Torrentio, for example) stores the same manifest under a
/// configuration-specific path. The host is the stable identity in that case.
pub(crate) fn addon_host(value: &str) -> String {
    let Ok(url) = Url::parse(value) else {
        return String::new();
    };
    url.host_str()
        .map(|host| host.trim_end_matches('.').to_ascii_lowercase())
        .unwrap_or_default()
}

fn addon_matches(left: &str, right: &str) -> bool {
    if addon_identity(left) == addon_identity(right) {
        return true;
    }
    let left_host = addon_host(left);
    !left_host.is_empty() && left_host == addon_host(right)
}

fn addon_types_label(descriptor: &Descriptor) -> String {
    let types = &descriptor.manifest.types;
    match types.as_slice() {
        [] => "Other".to_owned(),
        [only] => title_case_type(only),
        many => {
            let labels = many
                .iter()
                .map(|value| title_case_type(value))
                .collect::<Vec<_>>();
            format!(
                "{} & {}",
                labels[..labels.len() - 1].join(", "),
                labels.last().unwrap()
            )
        }
    }
}

fn title_case_type(value: &str) -> String {
    match value {
        "movie" => "Movie".to_owned(),
        "series" => "Series".to_owned(),
        "channel" | "tv" => "TV Channel".to_owned(),
        "anime" => "Anime".to_owned(),
        other => {
            let mut result = other.to_owned();
            if let Some(first) = result.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            result
        }
    }
}

fn project_addon(
    descriptor: &Descriptor,
    installed: bool,
    ui_weak: &slint::Weak<MainWindow>,
) -> AddonItem {
    let supports = |kind: &str| {
        descriptor
            .manifest
            .types
            .iter()
            .any(|value| value.eq_ignore_ascii_case(kind))
    };
    AddonItem {
        id: descriptor.manifest.id.as_str().into(),
        name: descriptor.manifest.name.as_str().into(),
        version: format!("v.{}", descriptor.manifest.version).into(),
        description: descriptor
            .manifest
            .description
            .clone()
            .unwrap_or_default()
            .into(),
        logo_url: descriptor
            .manifest
            .logo
            .as_ref()
            .map(Url::as_str)
            .unwrap_or_default()
            .into(),
        logo: crate::image_cache::get_poster_image(&descriptor.manifest.logo, ui_weak),
        is_installed: installed,
        transport_url: descriptor.transport_url.as_str().into(),
        configuration_url: descriptor
            .transport_url
            .as_str()
            .replace("manifest.json", "configure")
            .into(),
        types_label: addon_types_label(descriptor).into(),
        configurable: descriptor.manifest.behavior_hints.configurable,
        configuration_required: descriptor.manifest.behavior_hints.configuration_required,
        supports_movie: supports("movie"),
        supports_series: supports("series"),
        supports_anime: supports("anime"),
        supports_tv: supports("channel") || supports("tv"),
        background_url: descriptor
            .manifest
            .background
            .as_ref()
            .map(Url::as_str)
            .unwrap_or_default()
            .into(),
        background_image: crate::image_cache::get_poster_image(
            &descriptor.manifest.background,
            ui_weak,
        ),
        adult: descriptor.manifest.behavior_hints.adult,
        p2p: descriptor.manifest.behavior_hints.p2p,
    }
}

pub fn setup(
    ui: &MainWindow,
    runtime: &Arc<Runtime<DesktopEnv, AppModel>>,
    navigation: &NavigationController,
) {
    let ui_weak = ui.as_weak();

    {
        let runtime = runtime.clone();
        let ui_weak = ui_weak.clone();
        tokio::spawn(async move {
            let order = match DesktopEnv::get_storage::<Vec<String>>(ADDON_ORDER_STORAGE_KEY).await {
                Ok(Some(order)) => order,
                _ => match DesktopEnv::get_storage::<Vec<String>>(LEGACY_ADDON_ORDER_STORAGE_KEY).await {
                    Ok(Some(order)) => {
                        if let Err(error) = DesktopEnv::set_storage(ADDON_ORDER_STORAGE_KEY, Some(&order)).await {
                            tracing::warn!(%error, "failed to migrate addon ordering to SENZO storage");
                        }
                        order
                    }
                    _ => return,
                },
            };
            *PENDING_ADDON_ORDER
                .get_or_init(|| Mutex::new(Vec::new()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = order;

            let Some((remote_addons, installed)) = runtime.model().ok().map(|model| {
                (
                    model.remote_addons.clone(),
                    model.ctx.profile.addons.clone(),
                )
            }) else {
                return;
            };
            let ui_weak_for_sync = ui_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak.upgrade() {
                    sync(&ui, &remote_addons, &installed, &ui_weak_for_sync, &runtime);
                }
            });
        });
    }

    ui.on_addons_load_next_page({
        let runtime = runtime.clone();
        move || {
            let identity = runtime.model().ok().and_then(|model| {
                model
                    .remote_addons
                    .selectable
                    .next_page
                    .as_ref()
                    .and_then(|next_page| {
                        PaginationIdentity::new(
                            PaginationScope::Addons,
                            model
                                .remote_addons
                                .selected
                                .as_ref()
                                .map(|selected| &selected.request),
                            &next_page.request,
                        )
                    })
            });
            if pagination_gate().try_begin(identity) {
                runtime.dispatch(RuntimeAction {
                    field: Some(AppModelField::RemoteAddons),
                    action: Action::CatalogWithFilters(ActionCatalogWithFilters::LoadNextPage),
                });
            }
        }
    });

    // Install an add-on manifest into the local profile. Stremio core persists
    // this locally for unauthenticated users and syncs it only when an auth key
    // exists; the guest path must not require sign-in.
    ui.on_install_addon({
        let runtime = runtime.clone();
        let ui_weak = ui_weak.clone();
        move |manifest_url| {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_loading(true);
                ui.set_error_message("".into());
                ui.set_addons_install_feedback("Installing add-on…".into());
                ui.set_addons_install_feedback_error(false);
            }

            let rt = runtime.clone();
            let ui_weak = ui_weak.clone();
            let manifest_url = manifest_url.to_string();
            tokio::spawn(async move {
                let result = async {
                    let url = parse_manifest_url(&manifest_url)?;

                    // Copy the profile checks while the model read guard is scoped
                    // locally. Never retain the RwLock guard across the manifest fetch.
                    let (addons_locked, already_installed) = {
                        let model = rt
                            .model()
                            .map_err(|error| format!("Could not read the local profile: {error}"))?;
                        let profile = &model.ctx.profile;
                        (
                            profile.addons_locked,
                            profile
                                .addons
                                .iter()
                                .any(|addon| {
                                    addon_matches(addon.transport_url.as_str(), url.as_str())
                                }),
                        )
                    };
                    if addons_locked {
                        return Err(
                            "The add-on list is locked because its account sync failed. Re-enter Guest mode or sign in and retry."
                                .to_owned(),
                        );
                    }
                    if already_installed {
                        return Err("This add-on is already installed.".to_owned());
                    }

                    let request = http::Request::get(url.as_str())
                        .body(())
                        .map_err(|error| format!("Could not create manifest request: {error}"))?;
                    let manifest = DesktopEnv::fetch::<(), stremio_core::types::addon::Manifest>(
                        request,
                    )
                    .await
                    .map_err(|error| format!("Could not fetch the add-on manifest: {error}"))?;

                    if manifest.behavior_hints.configuration_required {
                        return Err(
                            "This add-on requires configuration before it can be installed."
                                .to_owned(),
                        );
                    }

                    let name = manifest.name.clone();
                    let descriptor = Descriptor {
                        manifest,
                        transport_url: url.clone(),
                        flags: Default::default(),
                    };
                    rt.dispatch(RuntimeAction {
                        field: None,
                        action: Action::Ctx(ActionCtx::InstallAddon(descriptor)),
                    });

                    let installed = rt
                        .model()
                        .ok()
                        .is_some_and(|model| {
                            model
                                .ctx
                                .profile
                                .addons
                                .iter()
                                .any(|addon| addon_matches(addon.transport_url.as_str(), url.as_str()))
                        });
                    if installed {
                        let installed_ui = ui_weak.clone();
                        let installed_url = url.to_string();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = installed_ui.upgrade() {
                                set_community_install_flag(&ui, &installed_url, true);
                            }
                        });
                        Ok(name)
                    } else {
                        Err("SENZO could not add this manifest to the local add-on list.".to_owned())
                    }
                }
                .await;

                match result {
                    Ok(name) => {
                        tracing::info!(addon = %name, "installed add-on into the local SENZO profile");
                        publish_install_feedback(
                            ui_weak,
                            format!("Installed {name}."),
                            false,
                        );
                    }
                    Err(message) => {
                        tracing::warn!(%message, "add-on installation failed");
                        publish_install_feedback(ui_weak, message, true);
                    }
                }
            });
        }
    });

    // Uninstall addon callback
    ui.on_uninstall_addon({
        let runtime = runtime.clone();
        let ui_weak = ui_weak.clone();
        move |transport_url| {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_loading(true);
            }
            let rt = runtime.clone();
            let transport_url = transport_url.to_string();
            let flag_weak = ui_weak.clone();
            tokio::spawn(async move {
                let model = rt.model().expect("model read failed");
                if let Ok(url) = Url::parse(&transport_url)
                    && let Some(descriptor) = model
                        .ctx
                        .profile
                        .addons
                        .iter()
                        .find(|a| {
                            addon_matches(a.transport_url.as_str(), url.as_str())
                        })
                {
                    let descriptor = descriptor.clone();
                    drop(model);
                    let removed_url = url.to_string();
                    let removed_ui = flag_weak.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = removed_ui.upgrade() {
                            set_community_install_flag(&ui, &removed_url, false);
                        }
                    });
                    rt.dispatch(RuntimeAction {
                        field: None,
                        action: Action::Ctx(ActionCtx::UninstallAddon(descriptor)),
                    });
                }
            });
        }
    });

    ui.on_reorder_addon({
        let runtime = runtime.clone();
        let ui_weak = ui_weak.clone();
        move |from, to| {
            let Some((mut profile, remote_addons)) = runtime
                .model()
                .ok()
                .map(|model| (model.ctx.profile.clone(), model.remote_addons.clone()))
            else {
                return;
            };

            {
                let mut pending = PENDING_ADDON_ORDER
                    .get_or_init(|| Mutex::new(Vec::new()))
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                apply_transport_order(&mut profile.addons, &pending);
                if !move_item(&mut profile.addons, from, to) {
                    return;
                }
                *pending = profile
                    .addons
                    .iter()
                    .map(|addon| addon.transport_url.to_string())
                    .collect();
            }

            if let Some(ui) = ui_weak.upgrade() {
                sync(&ui, &remote_addons, &profile.addons, &ui_weak, &runtime);
            }

            let runtime = runtime.clone();
            let ui_weak = ui_weak.clone();
            tokio::spawn(async move {
                let _reorder_guard = ADDON_REORDER_LOCK
                    .get_or_init(|| tokio::sync::Mutex::new(()))
                    .lock()
                    .await;

                let order = profile
                    .addons
                    .iter()
                    .map(|addon| addon.transport_url.to_string())
                    .collect::<Vec<_>>();
                if let Err(error) =
                    DesktopEnv::set_storage(ADDON_ORDER_STORAGE_KEY, Some(&order)).await
                {
                    tracing::error!(%error, "failed to persist addon order");
                }

                let Some(auth_key) = profile.auth_key().cloned() else {
                    let stored_profile = DesktopEnv::get_storage::<
                        stremio_core::types::profile::Profile,
                    >(PROFILE_STORAGE_KEY)
                    .await;
                    let mut stored_profile = match stored_profile {
                        Ok(Some(stored_profile)) => stored_profile,
                        Ok(None) => profile,
                        Err(error) => {
                            tracing::error!(%error, "failed to read the local addon profile");
                            return;
                        }
                    };
                    apply_transport_order(&mut stored_profile.addons, &order);
                    if let Err(error) =
                        DesktopEnv::set_storage(PROFILE_STORAGE_KEY, Some(&stored_profile)).await
                    {
                        tracing::error!(%error, "failed to persist the local addon profile");
                    }
                    return;
                };
                let request = APIRequest::AddonCollectionSet {
                    auth_key,
                    addons: profile.addons,
                };
                match fetch_api::<DesktopEnv, _, _, SuccessResponse>(&request).await {
                    Ok(APIResult::Ok(_)) => {
                        runtime.dispatch(RuntimeAction {
                            field: None,
                            action: Action::Ctx(ActionCtx::PullAddonsFromAPI),
                        });
                    }
                    Ok(APIResult::Err(error)) => {
                        tracing::error!(?error, "addon order was rejected by the API");
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_error_message(
                                    "Could not sync the new addon order to your account.".into(),
                                );
                            }
                        });
                    }
                    Err(error) => {
                        tracing::error!(%error, "failed to sync addon order");
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_error_message(
                                    "Could not sync the new addon order to your account.".into(),
                                );
                            }
                        });
                    }
                }
            });
        }
    });

    ui.on_open_addon_details({
        let runtime = runtime.clone();
        let ui_weak = ui_weak.clone();
        let navigation = navigation.clone();
        move |transport_url| {
            let Some(transport_url) = Url::parse(transport_url.as_str()).ok() else {
                return;
            };
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_addon_details_loading(true);
                ui.set_addon_details_error("".into());
                navigation.dispatch_and_project(
                    &ui,
                    NavigationIntent::OpenAddonDetails {
                        transport_url: transport_url.to_string(),
                    },
                );
            }
            runtime.dispatch(RuntimeAction {
                field: Some(AppModelField::AddonDetails),
                action: Action::Load(ActionLoad::AddonDetails(AddonDetailsSelected {
                    transport_url,
                })),
            });
        }
    });

    ui.on_close_addon_details({
        let runtime = runtime.clone();
        let ui_weak = ui_weak.clone();
        let navigation = navigation.clone();
        move || {
            if let Some(ui) = ui_weak.upgrade() {
                navigation.dispatch_and_project(&ui, NavigationIntent::Back);
            }
            runtime.dispatch(RuntimeAction {
                field: Some(AppModelField::AddonDetails),
                action: Action::Unload,
            });
        }
    });

    ui.on_configure_addon(move |configuration_url| {
        let Ok(configuration_url) = Url::parse(configuration_url.as_str()) else {
            tracing::warn!("refused invalid addon configuration URL");
            return;
        };
        if configuration_url.scheme() != "https"
            && configuration_url.host_str() != Some("localhost")
        {
            tracing::warn!("refused untrusted addon configuration URL");
            return;
        }
        if let Err(error) = open::that(configuration_url.as_str()) {
            tracing::error!(%error, "failed to open addon configuration");
        }
    });

    ui.on_share_addon(|transport_url| {
        match arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(transport_url.to_string()))
        {
            Ok(()) => tracing::info!(%transport_url, "addon link copied to clipboard"),
            Err(error) => tracing::error!(%error, "failed to copy addon link"),
        }
    });

    ui.on_addon_share_social(|platform, transport_url| {
        let base = match platform.as_str() {
            "facebook" => "https://www.facebook.com/sharer/sharer.php",
            "x" => "https://twitter.com/intent/tweet",
            "reddit" => "https://www.reddit.com/submit",
            _ => return,
        };
        let Ok(mut share_url) = Url::parse(base) else {
            return;
        };
        share_url
            .query_pairs_mut()
            .append_pair("url", transport_url.as_str());
        if let Err(error) = open::that(share_url.as_str()) {
            tracing::warn!(%error, %platform, "failed to open addon share URL");
        }
    });

    ui.on_addons_search_changed({
        let runtime = runtime.clone();
        let ui_weak = ui_weak.clone();
        move |_| {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let Ok(model) = runtime.model() else {
                return;
            };
            sync(
                &ui,
                &model.remote_addons,
                &model.ctx.profile.addons,
                &ui_weak,
                &runtime,
            );
        }
    });
}

#[tracing::instrument(skip_all)]
pub fn sync_details(ui: &MainWindow, details: &AddonDetails, ui_weak: &slint::Weak<MainWindow>) {
    let _span = tracing::info_span!("addon_details_mapping").entered();
    let mut loading = details.selected.is_some();
    let mut error = String::new();
    let mut descriptor = details.local_addon.as_ref();

    if let Some(remote) = details.remote_addon.as_ref() {
        match &remote.content {
            Loadable::Loading => loading = true,
            Loadable::Ready(remote_descriptor) => {
                loading = false;
                descriptor = Some(remote_descriptor);
            }
            Loadable::Err(load_error) => {
                loading = false;
                if descriptor.is_none() {
                    error = format!("Failed to load addon manifest: {load_error:?}");
                }
            }
        }
    }

    ui.set_addon_details_loading(loading);
    ui.set_addon_details_error(error.into());

    if let Some(descriptor) = descriptor {
        let installed = details.local_addon.is_some();
        ui.set_addon_details_addon(project_addon(descriptor, installed, ui_weak));
        ui.set_addon_details_configurable(descriptor.manifest.behavior_hints.configurable);
        ui.set_addon_details_configuration_required(
            descriptor.manifest.behavior_hints.configuration_required,
        );
    }
}

#[tracing::instrument(skip_all)]
pub fn sync(
    ui: &MainWindow,
    remote_addons: &CatalogWithFilters<Descriptor>,
    installed: &[Descriptor],
    ui_weak: &slint::Weak<MainWindow>,
    _runtime: &Arc<Runtime<DesktopEnv, AppModel>>,
) {
    let installed = ordered_installed(installed);
    let pagination_identity = remote_addons
        .selectable
        .next_page
        .as_ref()
        .and_then(|next_page| {
            PaginationIdentity::new(
                PaginationScope::Addons,
                remote_addons
                    .selected
                    .as_ref()
                    .map(|selected| &selected.request),
                &next_page.request,
            )
        });
    pagination_gate().observe(PaginationScope::Addons, pagination_identity);
    ui.set_addons_has_next_page(pagination_identity.is_some());

    let query = ui.get_addons_search_query().trim().to_lowercase();
    let mut fingerprint = Fingerprint::new();
    fingerprint.str(&query);
    for addon in &installed {
        hash_addon(&mut fingerprint, addon, true);
    }
    for page in &remote_addons.catalog {
        match &page.content {
            Some(Loadable::Ready(items)) => {
                fingerprint.u64(1);
                for addon in items {
                    hash_addon(&mut fingerprint, addon, false);
                }
            }
            _ => fingerprint.u64(0),
        }
    }
    if !sync_fingerprint_changed(&LAST_SYNC_STATE, fingerprint.finish()) {
        return;
    }

    let matches_query = |descriptor: &Descriptor| {
        query.is_empty()
            || descriptor.manifest.name.to_lowercase().contains(&query)
            || descriptor
                .manifest
                .description
                .as_deref()
                .unwrap_or_default()
                .to_lowercase()
                .contains(&query)
    };
    let estimated_count = {
        let _span = tracing::info_span!("filter_addon_catalogs").entered();
        installed.len()
            + remote_addons
                .catalog
                .iter()
                .filter_map(|page| {
                    if let Some(Loadable::Ready(items)) = &page.content {
                        Some(items.len())
                    } else {
                        None
                    }
                })
                .sum::<usize>()
    };

    let slint_addons = {
        let _span = tracing::info_span!("build_addon_items").entered();
        let mut slint_addons = Vec::with_capacity(estimated_count);
        let installed_urls = installed
            .iter()
            .map(|addon| addon.transport_url.as_str())
            .collect::<HashSet<_>>();

        // 1. Add all currently installed addons
        for addon in &installed {
            if matches_query(addon) {
                slint_addons.push(project_addon(addon, true, ui_weak));
            }
        }

        // 2. Add remote/discoverable addons that are not already installed
        for page in &remote_addons.catalog {
            if let Some(Loadable::Ready(items)) = &page.content {
                for addon in items {
                    // Avoid duplicating if already installed
                    if matches_query(addon)
                        && !installed_urls.contains(addon.transport_url.as_str())
                    {
                        slint_addons.push(project_addon(addon, false, ui_weak));
                    }
                }
            }
        }
        slint_addons
    };

    let addons_model = slint::VecModel::from(slint_addons);
    ui.set_addons_list(slint::ModelRc::new(addons_model));

    let installed_identities = installed
        .iter()
        .map(|addon| addon_identity(addon.transport_url.as_str()))
        .collect::<HashSet<_>>();
    let installed_hosts = installed
        .iter()
        .map(|addon| addon_host(addon.transport_url.as_str()))
        .filter(|host| !host.is_empty())
        .collect::<HashSet<_>>();
    sync_community_install_flags(ui, &installed_identities, &installed_hosts);
}

fn set_community_install_flag(ui: &MainWindow, manifest_url: &str, installed: bool) {
    let identity = addon_identity(manifest_url);
    let host = addon_host(manifest_url);
    let model = ui.get_addons_community_list();
    let mut items = (0..model.row_count())
        .filter_map(|index| model.row_data(index))
        .collect::<Vec<AddonItem>>();
    let mut changed = false;
    for item in &mut items {
        let same_addon = addon_identity(item.transport_url.as_str()) == identity
            || (!host.is_empty() && addon_host(item.transport_url.as_str()) == host);
        if same_addon && item.is_installed != installed {
            item.is_installed = installed;
            changed = true;
        }
    }
    if changed {
        ui.set_addons_community_list(slint::ModelRc::new(slint::VecModel::from(items)));
    }
}

fn sync_community_install_flags(
    ui: &MainWindow,
    installed: &HashSet<String>,
    installed_hosts: &HashSet<String>,
) {
    let model = ui.get_addons_community_list();
    let mut items = (0..model.row_count())
        .filter_map(|index| model.row_data(index))
        .collect::<Vec<AddonItem>>();
    if items.is_empty() {
        return;
    }
    let mut changed = false;
    for item in &mut items {
        let host = addon_host(item.transport_url.as_str());
        let is_installed = installed.contains(&addon_identity(item.transport_url.as_str()))
            || (!host.is_empty() && installed_hosts.contains(&host));
        if item.is_installed != is_installed {
            item.is_installed = is_installed;
            changed = true;
        }
    }
    if changed {
        ui.set_addons_community_list(slint::ModelRc::new(slint::VecModel::from(items)));
    }
}

#[cfg(test)]
mod tests {
    use super::{addon_host, addon_identity, addon_matches, move_item};

    #[test]
    fn addon_identity_ignores_configuration_queries_and_trailing_slashes() {
        assert_eq!(
            addon_identity("https://torrentio.strem.fun/manifest.json"),
            addon_identity("https://torrentio.strem.fun/manifest.json?debrid=realdebrid")
        );
        assert_eq!(
            addon_identity("https://Torrentio.Strem.Fun/manifest.json/"),
            addon_identity("https://torrentio.strem.fun/manifest.json")
        );
    }

    #[test]
    fn configured_manifest_matches_its_community_catalog_entry() {
        assert_eq!(
            addon_host("https://torrentio.strem.fun/realdebrid=abc/manifest.json"),
            "torrentio.strem.fun"
        );
        assert!(addon_matches(
            "https://torrentio.strem.fun/realdebrid=abc/manifest.json",
            "https://torrentio.strem.fun/manifest.json"
        ));
    }

    #[test]
    fn move_item_places_the_source_at_the_requested_index() {
        let mut items = vec!["one", "two", "three", "four"];

        move_item(&mut items, 0, 2);

        assert_eq!(items, vec!["two", "three", "one", "four"]);
    }

    #[test]
    fn move_item_rejects_an_out_of_bounds_target() {
        let mut items = vec![1, 2, 3];

        assert!(!move_item(&mut items, 1, 3));
    }
}

#[cfg(test)]
mod manifest_url_tests {
    use super::parse_manifest_url;

    #[test]
    fn accepts_http_and_https_addon_manifest_urls() {
        assert_eq!(
            parse_manifest_url("https://torrentio.strem.fun/manifest.json")
                .expect("HTTPS manifest URL")
                .scheme(),
            "https"
        );
        assert_eq!(
            parse_manifest_url("http://127.0.0.1:7000/manifest.json")
                .expect("local HTTP manifest URL")
                .host_str(),
            Some("127.0.0.1")
        );
    }

    #[test]
    fn rejects_non_http_manifest_urls() {
        assert!(parse_manifest_url("file:///tmp/manifest.json").is_err());
        assert!(parse_manifest_url("magnet:?xt=urn:btih:abc").is_err());
        assert!(parse_manifest_url("not a URL").is_err());
    }
}
