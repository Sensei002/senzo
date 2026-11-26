use std::{
    collections::HashMap,
    sync::{
        RwLock,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
};

use stremio_core::{
    models::{common::Loadable, meta_details::MetaDetails, player::Selected},
    types::{
        addon::{Descriptor, ResourcePath, ResourceRequest},
        resource::{
            Stream, StreamBehaviorHints, StreamProxyHeaders, StreamSource, Subtitles,
        },
    },
};

use crate::app_model::AppModel;

/// Presentation data for a stream whose full core selection remains in Rust.
#[derive(Clone, Debug)]
pub struct StreamSelectionView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub provider: String,
    /// `Direct` or `Add-on`. The details drawer groups filters by this value so
    /// the two source systems stay distinct without duplicating either list.
    pub source_system: String,
    pub thumbnail: Option<String>,
    pub progress: f32,
    pub score: i32,
    pub score_reasons: Vec<String>,
    pub filtered: bool,
    pub seeders: u32,
}

/// A Direct Sources result retained in the native playback selection table.
/// The URL has already passed provider validation and is re-validated by the
/// player path. Headers are authorized provider headers and are never logged.
#[derive(Clone, Debug)]
pub struct DirectSelection {
    pub id: String,
    pub provider_name: String,
    pub title: String,
    pub description: String,
    pub media_type: String,
    pub video_id: Option<String>,
    pub url: String,
    pub headers: std::collections::BTreeMap<String, String>,
    pub subtitles: Vec<String>,
}

/// A stream returned by an enabled Nuvio-compatible plugin. The plugin host
/// has already validated the URL/hash and sanitized the display fields.
#[derive(Clone, Debug)]
pub struct PluginSelection {
    pub id: String,
    pub provider_name: String,
    // The meta/video id this provider answered for. Plugin results used to
    // drop this, which left them without a stream_request and therefore
    // without the resume/progress key the player needs.
    pub video_id: Option<String>,
    pub title: String,
    pub description: String,
    pub url: Option<String>,
    pub info_hash: Option<String>,
    pub file_idx: Option<u16>,
    pub headers: Vec<(String, String)>,
    pub subtitles: Vec<String>,
    pub seeders: u32,
}

#[derive(Clone)]
struct RegisteredSelection {
    selected: Selected,
    stream_name: String,
}

fn stream_selection_id(resource_index: usize, stream_index: usize) -> String {
    format!("stream:{resource_index}:{stream_index}")
}

/// Keeps full core stream selections out of the Slint presentation model.
#[derive(Default)]
pub struct PlaybackSelections {
    entries: RwLock<HashMap<String, RegisteredSelection>>,
    trailer_id: RwLock<Option<String>>,
    direct: RwLock<Vec<DirectSelection>>,
    plugin: RwLock<Vec<PluginSelection>>,
    ranking_mode: AtomicU8,
    show_filtered: AtomicBool,
    source_generation: AtomicU64,
    source_key: RwLock<String>,
    debrid_availability: RwLock<HashMap<String, stream_ranking::DebridAvailability>>,
}

impl PlaybackSelections {
    pub fn clear(&self) {
        self.entries
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        *self
            .trailer_id
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        self.direct
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        self.plugin
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        self.debrid_availability
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        self.source_generation.fetch_add(1, Ordering::AcqRel);
    }

    /// Atomically replaces visible stream selections and returns their UI views.
    pub fn rebuild(
        &self,
        details: &MetaDetails,
        addons: &[Descriptor],
    ) -> Vec<StreamSelectionView> {
        let meta_request = details
            .selected
            .as_ref()
            .and_then(|selected| {
                details
                    .meta_items
                    .iter()
                    .find(|resource| resource.request.path.eq_no_extra(&selected.meta_path))
            })
            .map(|resource| resource.request.clone());
        let source_key = meta_request
            .as_ref()
            .map(|request| format!("{}:{}", request.path.r#type, request.path.id))
            .unwrap_or_default();
        let source_changed = {
            let mut current = self
                .source_key
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if *current == source_key {
                false
            } else {
                *current = source_key;
                true
            }
        };
        if source_changed {
            self.source_generation.fetch_add(1, Ordering::AcqRel);
            self.debrid_availability
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clear();
        }

        let mut next_entries = HashMap::new();
        let mut views = Vec::new();
        let mut rank_inputs = Vec::new();
        let provider_names: HashMap<&str, &str> = addons
            .iter()
            .map(|addon| (addon.transport_url.as_str(), addon.manifest.name.as_str()))
            .collect();

        let trailer_id = details
            .meta_items
            .iter()
            .find_map(|resource| {
                let Loadable::Ready(meta) = resource.content.as_ref()? else {
                    return None;
                };
                meta.preview.trailer_streams.first().cloned()
            })
            .map(|stream| {
                let id = "trailer".to_owned();
                next_entries.insert(
                    id.clone(),
                    RegisteredSelection {
                        selected: Selected {
                            stream,
                            stream_request: None,
                            meta_request: meta_request.clone(),
                            subtitles_path: None,
                        },
                        stream_name: "Trailer".to_owned(),
                    },
                );
                id
            });

        for (resource_index, resource) in details.streams.iter().enumerate() {
            let Some(Loadable::Ready(streams)) = &resource.content else {
                continue;
            };

            for (stream_index, stream) in streams.iter().enumerate() {
                // Stable IDs let the event loop skip replacing an unchanged
                // Slint stream model when an unrelated core field updates.
                let id = stream_selection_id(resource_index, stream_index);
                let name = stream.name.clone().unwrap_or_else(|| "Stream".to_owned());
                let description = stream.description.clone().unwrap_or_default();
                let subtitles_path = ResourcePath {
                    resource: "subtitles".to_owned(),
                    r#type: resource.request.path.r#type.clone(),
                    id: resource.request.path.id.clone(),
                    extra: Vec::new(),
                };
                let selected = Selected {
                    stream: stream.clone(),
                    stream_request: Some(resource.request.clone()),
                    meta_request: meta_request.clone(),
                    subtitles_path: Some(subtitles_path),
                };

                let seeders = stream
                    .behavior_hints
                    .other
                    .get("seeders")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|seeders| u32::try_from(seeders).ok())
                    .or_else(|| stream_ranking::parse_seeders_from_text(&description))
                    .or_else(|| stream_ranking::parse_seeders_from_text(&name));
                let size_bytes = stream
                    .behavior_hints
                    .video_size
                    .or_else(|| stream_ranking::parse_size_bytes_from_text(&description))
                    .or_else(|| stream_ranking::parse_size_bytes_from_text(&name));
                let formatted_description = stream_ranking::format_stream_description(
                    &name,
                    &description,
                    size_bytes,
                    seeders,
                );

                next_entries.insert(
                    id.clone(),
                    RegisteredSelection {
                        selected,
                        stream_name: if formatted_description.is_empty() {
                            name.clone()
                        } else {
                            formatted_description.clone()
                        },
                    },
                );
                views.push(StreamSelectionView {
                    id: id.clone(),
                    name: name.clone(),
                    description: formatted_description.clone(),
                    thumbnail: stream.thumbnail.clone(),
                    progress: stream
                        .behavior_hints
                        .other
                        .get("progress")
                        .and_then(serde_json::Value::as_f64)
                        .map(|progress| (progress / 100.0).clamp(0.0, 1.0) as f32)
                        .unwrap_or_default(),
                    provider: provider_names
                        .get(resource.request.base.as_str())
                        .map(|name| (*name).to_owned())
                        .unwrap_or_else(|| {
                            resource
                                .request
                                .base
                                .host_str()
                                .unwrap_or("Addon")
                                .to_owned()
                        }),
                    source_system: "Add-on".to_owned(),
                    score: 0,
                    score_reasons: Vec::new(),
                    filtered: false,
                    seeders: seeders.unwrap_or_default(),
                });
                rank_inputs.push(stream_ranking::RankInput {
                    id,
                    name: name.clone(),
                    description: formatted_description,
                    addon: resource.request.base.to_string(),
                    original_index: rank_inputs.len(),
                    size_bytes,
                    seeders,
                    debrid: self
                        .debrid_availability
                        .read()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .get(&stream_selection_id(resource_index, stream_index))
                        .copied(),
                });
            }
        }

        let ranked = stream_ranking::rank_streams(
            rank_inputs,
            self.ranking_mode(),
            self.show_filtered.load(Ordering::Acquire),
        );
        let ranking = ranked
            .into_iter()
            .enumerate()
            .map(|(position, ranked)| {
                let reasons = ranked
                    .reasons
                    .into_iter()
                    .map(|reason| {
                        let prefix = if reason.points > 0 { "+" } else { "" };
                        format!("{prefix}{} {}", reason.points, reason.label)
                    })
                    .collect::<Vec<_>>();
                (
                    ranked.input.id,
                    (position, ranked.score, reasons, ranked.filtered),
                )
            })
            .collect::<HashMap<_, _>>();
        views.retain(|view| ranking.contains_key(&view.id));
        for view in &mut views {
            if let Some((_, score, reasons, filtered)) = ranking.get(&view.id) {
                view.score = *score;
                view.score_reasons.clone_from(reasons);
                view.filtered = *filtered;
            }
        }
        views.sort_by_key(|view| {
            ranking
                .get(&view.id)
                .map(|rank| rank.0)
                .unwrap_or(usize::MAX)
        });

        // Direct Sources are appended after the ranked add-on list. They keep
        // their provider order and are never mixed into torrent ranking: the
        // two systems answer different questions and are filtered separately.
        let direct_selections = self
            .direct
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        for selection in direct_selections {
            let Ok(url) = url::Url::parse(&selection.url) else {
                continue;
            };
            let stream_request = selection.video_id.as_ref().and_then(|video_id| {
                url::Url::parse("https://direct.senzo.local/")
                    .ok()
                    .map(|base| ResourceRequest {
                        base,
                        path: ResourcePath {
                            resource: "stream".to_owned(),
                            r#type: selection.media_type.clone(),
                            id: video_id.clone(),
                            extra: Vec::new(),
                        },
                    })
            });
            let headers = selection.headers.iter().fold(
                HashMap::new(),
                |mut headers, (name, value)| {
                    headers.insert(name.clone(), value.clone());
                    headers
                },
            );
            let stream = Stream {
                source: StreamSource::Url { url },
                name: Some(selection.title.clone()),
                description: Some(selection.description.clone()),
                thumbnail: None,
                subtitles: selection
                    .subtitles
                    .iter()
                    .filter_map(|subtitle| url::Url::parse(subtitle).ok())
                    .map(|url| Subtitles {
                        id: url.to_string(),
                        lang: "und".to_owned(),
                        url,
                        label: None,
                    })
                    .collect(),
                behavior_hints: StreamBehaviorHints {
                    proxy_headers: (!headers.is_empty()).then_some(StreamProxyHeaders {
                        request: headers,
                        response: HashMap::new(),
                    }),
                    ..Default::default()
                },
            };
            next_entries.insert(
                selection.id.clone(),
                RegisteredSelection {
                    selected: Selected {
                        stream,
                        stream_request,
                        meta_request: meta_request.clone(),
                        subtitles_path: None,
                    },
                    stream_name: selection.title.clone(),
                },
            );
            views.push(StreamSelectionView {
                id: selection.id,
                name: selection.title,
                description: selection.description,
                provider: selection.provider_name,
                source_system: "Direct".to_owned(),
                thumbnail: None,
                progress: 0.0,
                score: 0,
                score_reasons: Vec::new(),
                filtered: false,
                seeders: 0,
            });
        }

        // Plugin results use the same drawer and MPV path as Direct results,
        // but stay in their own source_system so filters remain honest.
        let plugin_selections = self
            .plugin
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        for selection in plugin_selections {
            let info_hash = selection.info_hash.as_deref().and_then(parse_info_hash);
            let source = if let Some(info_hash) = info_hash {
                StreamSource::Torrent {
                    info_hash,
                    file_idx: selection.file_idx,
                    announce: magnet_trackers(selection.url.as_deref()),
                    file_must_include: Vec::new(),
                }
            } else if let Some(url) = selection.url.as_deref().and_then(|url| url::Url::parse(url).ok()) {
                StreamSource::Url { url }
            } else {
                continue;
            };
            let headers = selection.headers.iter().fold(
                HashMap::new(),
                |mut headers, (name, value)| {
                    headers.insert(name.clone(), value.clone());
                    headers
                },
            );
            let stream_request = selection.video_id.as_ref().and_then(|video_id| {
                let meta_request = meta_request.as_ref()?;
                Some(ResourceRequest {
                    base: meta_request.base.clone(),
                    path: ResourcePath {
                        resource: "stream".to_owned(),
                        r#type: meta_request.path.r#type.clone(),
                        id: video_id.clone(),
                        extra: Vec::new(),
                    },
                })
            });
            let stream = Stream {
                source,
                name: Some(selection.title.clone()),
                description: Some(selection.description.clone()),
                thumbnail: None,
                subtitles: selection
                    .subtitles
                    .iter()
                    .filter_map(|subtitle| url::Url::parse(subtitle).ok())
                    .map(|url| Subtitles {
                        id: url.to_string(),
                        lang: "und".to_owned(),
                        url,
                        label: None,
                    })
                    .collect(),
                behavior_hints: StreamBehaviorHints {
                    proxy_headers: (!headers.is_empty()).then_some(StreamProxyHeaders {
                        request: headers,
                        response: HashMap::new(),
                    }),
                    ..Default::default()
                },
            };
            next_entries.insert(
                selection.id.clone(),
                RegisteredSelection {
                    selected: Selected {
                        stream,
                        stream_request,
                        meta_request: meta_request.clone(),
                        subtitles_path: None,
                    },
                    stream_name: selection.title.clone(),
                },
            );
            views.push(StreamSelectionView {
                id: selection.id,
                name: selection.title,
                description: selection.description,
                provider: selection.provider_name,
                source_system: "Plugin".to_owned(),
                thumbnail: None,
                progress: 0.0,
                score: 0,
                score_reasons: Vec::new(),
                filtered: false,
                seeders: selection.seeders,
            });
        }

        match self.entries.write() {
            Ok(mut entries) => *entries = next_entries,
            Err(poisoned) => *poisoned.into_inner() = next_entries,
        }
        match self.trailer_id.write() {
            Ok(mut current) => *current = trailer_id,
            Err(poisoned) => *poisoned.into_inner() = trailer_id,
        }
        views
    }

    pub fn source_generation(&self) -> u64 {
        self.source_generation.load(Ordering::Acquire)
    }

    pub fn trailer_id(&self) -> Option<String> {
        match self.trailer_id.read() {
            Ok(id) => id.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    pub fn set_direct_results(&self, selections: Vec<DirectSelection>) {
        *self
            .direct
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = selections;
    }

    pub fn clear_direct_results(&self) {
        self.direct
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }

    pub fn set_plugin_results(&self, selections: Vec<PluginSelection>) {
        *self
            .plugin
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = selections;
    }

    pub fn clear_plugin_results(&self) {
        self.plugin
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }

    pub fn set_ranking_mode(&self, mode: stream_ranking::RankingMode) {
        self.ranking_mode
            .store(ranking_mode_byte(mode), Ordering::Release);
    }

    pub fn ranking_mode(&self) -> stream_ranking::RankingMode {
        match self.ranking_mode.load(Ordering::Acquire) {
            1 => stream_ranking::RankingMode::Quality,
            2 => stream_ranking::RankingMode::Smallest,
            3 => stream_ranking::RankingMode::Seeders,
            4 => stream_ranking::RankingMode::Original,
            _ => stream_ranking::RankingMode::Smart,
        }
    }

    pub fn set_show_filtered(&self, show: bool) {
        self.show_filtered.store(show, Ordering::Release);
    }

    pub fn debrid_candidates(&self) -> (u64, Vec<(String, String)>) {
        let availability = self
            .debrid_availability
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entries = self
            .entries
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let candidates = entries
            .iter()
            .filter(|(id, _)| !availability.contains_key(*id))
            .filter_map(|(id, entry)| match &entry.selected.stream.source {
                stremio_core::types::resource::StreamSource::Torrent { info_hash, .. } => {
                    Some((id.clone(), hex::encode(info_hash)))
                }
                _ => None,
            })
            .collect();
        (self.source_generation.load(Ordering::Acquire), candidates)
    }

    pub fn apply_debrid_availability(
        &self,
        generation: u64,
        availability: HashMap<String, stream_ranking::DebridAvailability>,
    ) -> bool {
        if self.source_generation.load(Ordering::Acquire) != generation {
            return false;
        }
        self.debrid_availability
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .extend(availability);
        true
    }

    /// Resolves an opaque UI ID back to the full core selection and label.
    pub fn resolve(&self, id: &str) -> Option<(Selected, String)> {
        let entries = match self.entries.read() {
            Ok(entries) => entries,
            Err(poisoned) => poisoned.into_inner(),
        };
        entries
            .get(id)
            .map(|entry| (entry.selected.clone(), entry.stream_name.clone()))
    }
}
fn ranking_mode_byte(mode: stream_ranking::RankingMode) -> u8 {
    match mode {
        stream_ranking::RankingMode::Smart => 0,
        stream_ranking::RankingMode::Quality => 1,
        stream_ranking::RankingMode::Smallest => 2,
        stream_ranking::RankingMode::Seeders => 3,
        stream_ranking::RankingMode::Original => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::stream_selection_id;

    #[test]
    fn stream_selection_ids_are_stable_and_resource_scoped() {
        assert_eq!(stream_selection_id(2, 7), "stream:2:7");
        assert_ne!(stream_selection_id(1, 0), stream_selection_id(0, 1));
    }
}

fn parse_info_hash(value: &str) -> Option<[u8; 20]> {
    let decoded = hex::decode(value.trim()).ok()?;
    let hash: [u8; 20] = decoded.try_into().ok()?;
    Some(hash)
}

fn magnet_trackers(value: Option<&str>) -> Vec<String> {
    let Some(value) = value.and_then(|value| value.strip_prefix("magnet:?")) else {
        return Vec::new();
    };
    url::form_urlencoded::parse(value.as_bytes())
        .filter(|(key, _)| key.as_ref() == "tr")
        .map(|(_, value)| value.into_owned())
        .filter(|tracker| tracker.starts_with("http://") || tracker.starts_with("https://") || tracker.starts_with("udp://"))
        .take(32)
        .collect()
}
