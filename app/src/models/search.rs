use crate::models::details::{load_meta_details_for_video, open_details_route};
use crate::models::{
    CatalogScope, Fingerprint, SyncFingerprint, catalog_name_index, fingerprint_catalog_projection,
    project_catalog_section, queue_visible_catalog, spawn_catalog_visibility_loader,
    sync_fingerprint_changed,
};
use crate::{
    AppModel, AppModelField, MainWindow, NavigationController, NavigationIntent, SearchSuggestion,
};
use core_env::DesktopEnv;
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex, OnceLock},
};
use stremio_core::{
    models::{
        catalogs_with_extra::{CatalogsWithExtra, Selected},
        common::Loadable,
        local_search::LocalSearch,
    },
    runtime::{
        Runtime, RuntimeAction,
        msg::{Action, ActionCatalogsWithExtra, ActionLoad, ActionSearch},
    },
    types::addon::{Descriptor, ExtraValue},
};

static LAST_LOCAL_SYNC_STATE: OnceLock<Mutex<Option<SyncFingerprint>>> = OnceLock::new();
static LAST_RESULTS_SYNC_STATE: OnceLock<Mutex<Option<SyncFingerprint>>> = OnceLock::new();
const LOCAL_SEARCH_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(250);

pub fn setup(
    ui: &MainWindow,
    runtime: &Arc<Runtime<DesktopEnv, AppModel>>,
    navigation: &NavigationController,
) {
    let catalog_loader = spawn_catalog_visibility_loader(runtime, CatalogScope::Search);
    let pending_local_search = Arc::new(Mutex::new(None::<tokio::task::JoinHandle<()>>));
    ui.on_search_catalog_visible(move |index| {
        queue_visible_catalog(&catalog_loader, index);
    });

    ui.on_global_search_edited({
        let runtime = runtime.clone();
        let pending_local_search = Arc::clone(&pending_local_search);
        move |query| {
            let mut pending = pending_local_search
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(task) = pending.take() {
                task.abort();
            }
            let runtime = runtime.clone();
            let query = query.trim().to_owned();
            *pending = Some(tokio::spawn(async move {
                tokio::time::sleep(LOCAL_SEARCH_DEBOUNCE).await;
                runtime.dispatch(RuntimeAction {
                    field: Some(AppModelField::LocalSearch),
                    action: Action::Search(ActionSearch::Search {
                        search_query: query,
                        max_results: 5,
                    }),
                });
            }));
        }
    });

    ui.on_global_search_submitted({
        let runtime = runtime.clone();
        let ui_weak = ui.as_weak();
        let navigation = navigation.clone();
        let pending_local_search = Arc::clone(&pending_local_search);
        move |query| {
            cancel_pending_local_search(&pending_local_search);
            if let Some(ui) = ui_weak.upgrade() {
                submit_search(&ui, &runtime, &navigation, query.as_str());
            }
        }
    });

    ui.on_global_search_suggestion_selected({
        let runtime = runtime.clone();
        let ui_weak = ui.as_weak();
        let navigation = navigation.clone();
        let pending_local_search = Arc::clone(&pending_local_search);
        move |query| {
            cancel_pending_local_search(&pending_local_search);
            if let Some(ui) = ui_weak.upgrade() {
                submit_search(&ui, &runtime, &navigation, query.as_str());
            }
        }
    });

    ui.on_search_item_selected({
        let runtime = runtime.clone();
        let ui_weak = ui.as_weak();
        let navigation = navigation.clone();
        move |id, media_type, video_id| {
            if let Some(ui) = ui_weak.upgrade() {
                let video_id = (!video_id.is_empty()).then_some(video_id);
                open_details(&ui, &runtime, &navigation, id, media_type, video_id);
            }
        }
    });
}

fn cancel_pending_local_search(pending: &Mutex<Option<tokio::task::JoinHandle<()>>>) {
    if let Some(task) = pending
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
    {
        task.abort();
    }
}

fn submit_search(
    ui: &MainWindow,
    runtime: &Arc<Runtime<DesktopEnv, AppModel>>,
    navigation: &NavigationController,
    query: &str,
) {
    let query = query.trim().to_owned();
    if query.is_empty() {
        return;
    }

    if query.starts_with("magnet:") || query.starts_with("stremio:") {
        crate::deep_link::handle(
            crate::single_instance::AppCommand::Open(query),
            ui,
            runtime,
            navigation,
        );
        return;
    }

    ui.set_search_query(query.as_str().into());
    ui.set_search_suggestions(ModelRc::new(VecModel::from(Vec::new())));
    ui.set_search_sections(ModelRc::new(VecModel::from(Vec::new())));
    ui.set_search_catalog_count(0);
    ui.set_search_loading(true);
    navigation.dispatch_and_project(
        ui,
        NavigationIntent::OpenSearch {
            query: query.clone(),
        },
    );

    for action in search_reset_actions(query) {
        runtime.dispatch(action);
    }
}

/// Clear the previous search before loading the new one.
///
/// Core reuses a planned catalog whose request matches and whose content is
/// already present, and it treats that as "no change". Re-submitting the query
/// that is already loaded (or one whose request errored) therefore produced no
/// state at all, while the UI had just been cleared and marked as loading. The
/// unload guarantees a fresh plan plus a real state change on every submit, so
/// the same search always runs again.
fn search_reset_actions(query: String) -> Vec<RuntimeAction<DesktopEnv, AppModel>> {
    vec![
        RuntimeAction {
            field: Some(AppModelField::Search),
            action: Action::Unload,
        },
        RuntimeAction {
            field: Some(AppModelField::Search),
            action: Action::Load(ActionLoad::CatalogsWithExtra(Selected {
                r#type: None,
                extra: vec![ExtraValue {
                    name: "search".to_owned(),
                    value: query,
                }],
            })),
        },
    ]
}

fn open_details(
    ui: &MainWindow,
    runtime: &Arc<Runtime<DesktopEnv, AppModel>>,
    navigation: &NavigationController,
    id: slint::SharedString,
    media_type: slint::SharedString,
    video_id: Option<slint::SharedString>,
) {
    let id = id.to_string();
    ui.set_search_suggestions(ModelRc::new(VecModel::from(Vec::new())));
    open_details_route(ui, runtime, navigation, &id);
    load_meta_details_for_video(
        runtime,
        id,
        Some(media_type.to_string()),
        video_id.map(|value| value.to_string()),
    );
}

pub fn sync_local_search(
    ui: &MainWindow,
    local_search: &LocalSearch,
    ui_weak: &slint::Weak<MainWindow>,
) {
    let _span = tracing::info_span!("sync_local_search_suggestions").entered();
    let mut seen_queries = HashSet::with_capacity(local_search.search_results.len());
    let unique_results = local_search
        .search_results
        .iter()
        .filter(|item| seen_queries.insert(item.name.as_str()))
        .collect::<Vec<_>>();

    let mut fingerprint = Fingerprint::new();
    for item in &unique_results {
        fingerprint.str(&item.id);
        fingerprint.str(&item.r#type);
        fingerprint.str(&item.name);
        fingerprint.optional_str(item.release_info.as_deref());
        fingerprint.optional_str(item.poster.as_ref().map(url::Url::as_str));
    }
    if !sync_fingerprint_changed(&LAST_LOCAL_SYNC_STATE, fingerprint.finish()) {
        return;
    }

    let suggestions = unique_results
        .iter()
        .map(|item| SearchSuggestion {
            id: item.id.as_str().into(),
            media_type: item.r#type.as_str().into(),
            title: item.name.as_str().into(),
            release_info: item.release_info.as_deref().unwrap_or_default().into(),
            poster_url: item
                .poster
                .as_ref()
                .map(url::Url::as_str)
                .unwrap_or_default()
                .into(),
            poster: crate::image_cache::get_poster_image(&item.poster, ui_weak),
        })
        .collect::<Vec<_>>();
    ui.set_search_suggestions(ModelRc::new(VecModel::from(suggestions)));
}

pub fn sync_results(
    ui: &MainWindow,
    search: &CatalogsWithExtra,
    addons: &[Descriptor],
    runtime: &Arc<Runtime<DesktopEnv, AppModel>>,
    _ui_weak: &slint::Weak<MainWindow>,
) {
    let catalog_count = search.catalogs.len();
    let loading = search.catalogs.iter().any(|catalog| {
        catalog.first().is_none_or(|page| {
            page.content
                .as_ref()
                .is_none_or(|content| matches!(content, Loadable::Loading))
        })
    });

    // A range load can leave a planned catalog without content, and Core never
    // reuses or retries such a page. Search renders every catalog, so a page
    // without content never produces a row, and without a row no visibility
    // callback can ask for it again: the page would keep showing skeletons
    // forever. Ask for every planned catalog instead, which only re-requests
    // the empty ones.
    ensure_catalogs_requested(search, runtime);

    let mut fingerprint = Fingerprint::new();
    fingerprint.usize(catalog_count);
    fingerprint.bool(loading);
    for addon in addons {
        fingerprint.str(addon.transport_url.as_str());
        for catalog in &addon.manifest.catalogs {
            fingerprint.str(&catalog.id);
            fingerprint.str(&catalog.r#type);
            fingerprint.optional_str(catalog.name.as_deref());
        }
    }
    fingerprint_catalog_projection(&mut fingerprint, search);
    if !sync_fingerprint_changed(&LAST_RESULTS_SYNC_STATE, fingerprint.finish()) {
        return;
    }

    ui.set_search_catalog_count(i32::try_from(catalog_count).unwrap_or(i32::MAX));
    ui.set_search_loading(loading);

    let mut sections = Vec::with_capacity(search.catalogs.len());
    {
        let _span = tracing::info_span!("map_search_sections").entered();
        let catalog_names = catalog_name_index(addons);
        for (catalog_index, catalog) in search.catalogs.iter().enumerate() {
            let Some(page) = catalog.first() else {
                continue;
            };
            let catalog_name = catalog_names
                .get(&(
                    page.request.base.as_str(),
                    page.request.path.id.as_str(),
                    page.request.path.r#type.as_str(),
                ))
                .copied()
                .unwrap_or(page.request.path.id.as_str());
            if let Some(section) = project_catalog_section(catalog_index, catalog, catalog_name) {
                sections.push(section);
            }
        }
    }

    // Update rows in-place when the section count is stable so the ListView
    // keeps its current viewport-y. A full model replacement resets scroll
    // position, which causes a visible snap-back during lazy catalog loading.
    let existing = ui.get_search_sections();
    if existing.row_count() == sections.len() {
        for (index, section) in sections.into_iter().enumerate() {
            existing.set_row_data(index, section);
        }
    } else {
        ui.set_search_sections(ModelRc::new(VecModel::from(sections)));
    }
}

/// Catalogs that Core planned but never requested.
fn catalogs_without_content(search: &CatalogsWithExtra) -> usize {
    search
        .catalogs
        .iter()
        .filter(|catalog| catalog.first().is_none_or(|page| page.content.is_none()))
        .count()
}

/// Request every planned catalog that has no content yet.
///
/// A ranged load only requests the catalogs inside the range; the rest are
/// planned with no content and no effect, and Core never reuses or retries such
/// a page on its own. The search page renders one row per catalog that has
/// content, so an empty page produces no row, and without a row no visibility
/// callback can ask for it again. Asking for the whole range only re-requests
/// the empty pages (the reuse rule keeps the loaded ones), which closes that
/// deadlock instead of leaving the page on skeletons forever.
fn ensure_catalogs_requested(
    search: &CatalogsWithExtra,
    runtime: &Arc<Runtime<DesktopEnv, AppModel>>,
) {
    if search.selected.is_none() || search.catalogs.is_empty() {
        return;
    }
    let missing = catalogs_without_content(search);
    if missing == 0 {
        return;
    }
    tracing::info!(
        missing_catalogs = missing,
        catalog_count = search.catalogs.len(),
        "requesting search catalogs that were planned without content"
    );
    runtime.dispatch(RuntimeAction {
        field: Some(AppModelField::Search),
        action: Action::CatalogsWithExtra(ActionCatalogsWithExtra::LoadRange(
            0..search.catalogs.len(),
        )),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use stremio_core::{
        models::common::ResourceLoadable,
        types::{addon::{ResourcePath, ResourceRequest}, resource::MetaItemPreview},
    };

    fn request() -> ResourceRequest {
        ResourceRequest {
            base: url::Url::parse("https://example.com/manifest.json").expect("base url"),
            path: ResourcePath {
                id: "search".to_owned(),
                r#type: "movie".to_owned(),
                resource: "catalog".to_owned(),
                extra: Vec::new(),
            },
        }
    }

    fn search_with(contents: Vec<Option<Loadable<Vec<MetaItemPreview>>>>) -> CatalogsWithExtra {
        CatalogsWithExtra {
            selected: Some(Selected {
                r#type: None,
                extra: vec![ExtraValue {
                    name: "search".to_owned(),
                    value: "taandob".to_owned(),
                }],
            }),
            catalogs: contents
                .into_iter()
                .map(|content| vec![ResourceLoadable { request: request(), content }])
                .collect(),
        }
    }

    #[test]
    fn re_submitting_a_search_unloads_before_loading_again() {
        let actions = search_reset_actions("taandob".to_owned());
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].field, Some(AppModelField::Search));
        assert!(matches!(actions[0].action, Action::Unload));
        match &actions[1].action {
            Action::Load(ActionLoad::CatalogsWithExtra(selected)) => {
                assert_eq!(selected.r#type, None);
                assert_eq!(selected.extra.len(), 1);
                assert_eq!(selected.extra[0].name, "search");
                assert_eq!(selected.extra[0].value, "taandob");
            }
            other => panic!("expected a catalogs-with-extra load, got {other:?}"),
        }
    }

    #[test]
    fn planned_catalogs_without_content_are_detected() {
        let search = search_with(vec![
            None,
            Some(Loadable::Loading),
            Some(Loadable::Err("boom".to_owned())),
            Some(Loadable::Ready(Vec::new())),
        ]);
        assert_eq!(catalogs_without_content(&search), 1);
    }

    #[test]
    fn loaded_catalogs_never_trigger_another_request() {
        let search = search_with(vec![
            Some(Loadable::Loading),
            Some(Loadable::Ready(Vec::new())),
        ]);
        assert_eq!(catalogs_without_content(&search), 0);
    }
}
