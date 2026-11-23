//! QuickJS execution host for Nuvio provider scripts.
//!
//! One runtime is created per provider query on a blocking worker thread. The
//! runtime is memory-capped, stack-capped, and interrupted after a deadline.
//! Native functions are intentionally narrow: fetch, sleeping timers, URL
//! parsing, text encoding, RustCrypto, and an HTML selector facade.

use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use rquickjs::{Context, Ctx, Function, Runtime};
use serde_json::{Value, json};

use super::{
    PluginStream,
    crypto,
    dom::{self, DomStore},
    net::{FetchRequest, SafeHttp},
};

const MEMORY_LIMIT: usize = 128 * 1024 * 1024;
const STACK_LIMIT: usize = 2 * 1024 * 1024;
const EXECUTION_DEADLINE: Duration = Duration::from_secs(60);
const MAX_RESULT_STREAMS: usize = 48;
const MAX_LOG_LINES: usize = 80;

const PRELUDE: &str = r#"
var module = { exports: {} };
var exports = module.exports;
"#;

const CALL_CODE: &str = r#"
(async function () {
  try {
    var getStreams = (typeof module !== 'undefined' && module.exports && module.exports.getStreams)
      || globalThis.getStreams;
    if (typeof getStreams !== 'function') {
      throw new Error('getStreams is not exported by this provider');
    }
    var args = JSON.parse(__senzo_call_args());
    var season = args.season == null ? undefined : args.season;
    var episode = args.episode == null ? undefined : args.episode;
    var result = await getStreams(args.tmdbId, args.mediaType, season, episode);
    __senzo_capture_result(JSON.stringify(result || []));
  } catch (error) {
    __senzo_capture_error(error && error.message ? error.message : String(error));
  }
})();
"#;

const CHECK_CODE: &str = r#"
(function () {
  var fn = (typeof module !== 'undefined' && module.exports && module.exports.getStreams)
    || globalThis.getStreams;
  return typeof fn === 'function';
})()
"#;

pub(super) struct ProviderRequest {
    pub tmdb_id: String,
    pub media_type: String,
    pub season: Option<u32>,
    pub episode: Option<u32>,
}

pub struct ExecutionOutcome {
    pub streams: Vec<PluginStream>,
    pub error: Option<String>,
}

pub(super) fn execute(
    provider_key: &str,
    provider_name: &str,
    script: &str,
    settings: &str,
    request: &ProviderRequest,
) -> ExecutionOutcome {
    let logs = Rc::new(RefCell::new(Vec::<String>::new()));
    let result = run(provider_key, provider_name, script, settings, request, logs);
    match result {
        Ok(streams) => ExecutionOutcome {
            streams,
            error: None,
        },
        Err(error) => ExecutionOutcome {
            streams: Vec::new(),
            error: Some(format!("{provider_name}: {error}")),
        },
    }
}

pub(super) fn validate(provider_key: &str, script: &str, settings: &str) -> Result<(), String> {
    let logs = Rc::new(RefCell::new(Vec::<String>::new()));
    let runtime = build_runtime()?;
    let context = Context::full(&runtime).map_err(|_| "Could not create the plugin runtime.".to_owned())?;
    context.with(|ctx| {
        install(
            &ctx,
            provider_key,
            script,
            settings,
            &ProviderRequest {
                tmdb_id: "0".to_owned(),
                media_type: "movie".to_owned(),
                season: None,
                episode: None,
            },
            Rc::new(DomStore::default()),
            logs,
            Rc::new(RefCell::new(None)),
            Rc::new(RefCell::new(None)),
            true,
        )?;
        let found: bool = ctx
            .eval(CHECK_CODE)
            .map_err(|_| "The provider did not expose a usable getStreams function.".to_owned())?;
        if found {
            Ok(())
        } else {
            Err("The provider did not expose a usable getStreams function.".to_owned())
        }
    })
}

fn run(
    provider_key: &str,
    provider_name: &str,
    script: &str,
    settings: &str,
    request: &ProviderRequest,
    logs: Rc<RefCell<Vec<String>>>,
) -> Result<Vec<PluginStream>, String> {
    let runtime = build_runtime()?;
    let context = Context::full(&runtime)
        .map_err(|_| "Could not create the plugin runtime.".to_owned())?;
    context.with(|ctx| {
        let captured = Rc::new(RefCell::new(None::<String>));
        let captured_error = Rc::new(RefCell::new(None::<String>));
        install(
            &ctx,
            provider_key,
            script,
            settings,
            request,
            Rc::new(DomStore::default()),
            logs,
            captured.clone(),
            captured_error.clone(),
            true,
        )?;

        ctx.eval::<(), _>(CALL_CODE)
            .map_err(|_| "The provider could not be started.".to_owned())?;
        // QuickJS has no event loop. Every native bridge resolves immediately,
        // so draining the microtask queue is enough to finish the async
        // getStreams call.
        for _ in 0..100_000 {
            if !ctx.execute_pending_job() {
                break;
            }
        }

        if let Some(error) = captured_error.borrow().clone() {
            return Err(truncate(&error, 300));
        }
        let Some(value) = captured.borrow().clone() else {
            return Err("The provider did not return a result.".to_owned());
        };
        parse_streams(provider_key, provider_name, &value)
    })
}

fn build_runtime() -> Result<Runtime, String> {
    let runtime = Runtime::new().map_err(|_| "Could not initialize QuickJS.".to_owned())?;
    runtime.set_memory_limit(MEMORY_LIMIT);
    runtime.set_max_stack_size(STACK_LIMIT);
    let deadline = Instant::now() + EXECUTION_DEADLINE;
    runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)));
    Ok(runtime)
}

#[allow(clippy::too_many_arguments)]
fn install<'js>(
    ctx: &Ctx<'js>,
    provider_key: &str,
    script: &str,
    settings: &str,
    request: &ProviderRequest,
    dom_store: Rc<DomStore>,
    logs: Rc<RefCell<Vec<String>>>,
    captured: Rc<RefCell<Option<String>>>,
    captured_error: Rc<RefCell<Option<String>>>,
    execute_script: bool,
) -> Result<(), String> {
    let globals = ctx.globals();
    let http = Rc::new(SafeHttp::new());

    let scraper_id = provider_key.to_owned();
    globals
        .set(
            "__senzo_scraper_id",
            Function::new(ctx.clone(), move || scraper_id.clone())
                .map_err(|_| "Could not install the plugin bridge.".to_owned())?,
        )
        .map_err(|_| "Could not install the plugin bridge.".to_owned())?;
    let settings_json = settings.to_owned();
    globals
        .set(
            "__senzo_settings_json",
            Function::new(ctx.clone(), move || settings_json.clone())
                .map_err(|_| "Could not install the plugin bridge.".to_owned())?,
        )
        .map_err(|_| "Could not install the plugin bridge.".to_owned())?;
    let call_args = json!({
        "tmdbId": request.tmdb_id,
        "mediaType": request.media_type,
        "season": request.season,
        "episode": request.episode,
    })
    .to_string();
    globals
        .set(
            "__senzo_call_args",
            Function::new(ctx.clone(), move || call_args.clone())
                .map_err(|_| "Could not install the plugin bridge.".to_owned())?,
        )
        .map_err(|_| "Could not install the plugin bridge.".to_owned())?;

    let result_slot = captured.clone();
    globals
        .set(
            "__senzo_capture_result",
            Function::new(ctx.clone(), move |value: String| {
                *result_slot.borrow_mut() = Some(value);
            })
            .map_err(|_| "Could not install the plugin bridge.".to_owned())?,
        )
        .map_err(|_| "Could not install the plugin bridge.".to_owned())?;
    let error_slot = captured_error.clone();
    globals
        .set(
            "__senzo_capture_error",
            Function::new(ctx.clone(), move |value: String| {
                *error_slot.borrow_mut() = Some(value);
            })
            .map_err(|_| "Could not install the plugin bridge.".to_owned())?,
        )
        .map_err(|_| "Could not install the plugin bridge.".to_owned())?;

    let log_slot = logs.clone();
    let log_name = provider_key.to_owned();
    globals
        .set(
            "__senzo_log",
            Function::new(ctx.clone(), move |level: String, message: String| {
                let message = redact_log(&truncate(&message, 800));
                tracing::debug!(plugin = %log_name, level = %level, "{message}");
                let mut lines = log_slot.borrow_mut();
                if lines.len() < MAX_LOG_LINES {
                    lines.push(format!("{level}: {message}"));
                }
            })
            .map_err(|_| "Could not install the plugin bridge.".to_owned())?,
        )
        .map_err(|_| "Could not install the plugin bridge.".to_owned())?;

    let fetch_http = http.clone();
    globals
        .set(
            "__senzo_fetch",
            Function::new(ctx.clone(), move |request: String| -> rquickjs::Result<String> {
                let request = serde_json::from_str::<FetchRequest>(&request)
                    .map_err(|_| rquickjs::Error::new_from_js_message("plugin", "fetch", "The provider sent an invalid fetch request."))?;
                serde_json::to_string(&fetch_http.fetch(request))
                    .map_err(|_| rquickjs::Error::new_from_js_message("plugin", "fetch", "Could not encode the fetch response."))
            })
            .map_err(|_| "Could not install the plugin bridge.".to_owned())?,
        )
        .map_err(|_| "Could not install the plugin bridge.".to_owned())?;
    globals
        .set(
            "__senzo_sleep",
            Function::new(ctx.clone(), |milliseconds: u64| {
                std::thread::sleep(Duration::from_millis(milliseconds.min(60_000)));
            })
            .map_err(|_| "Could not install the plugin bridge.".to_owned())?,
        )
        .map_err(|_| "Could not install the plugin bridge.".to_owned())?;
    globals
        .set(
            "__senzo_parse_url",
            Function::new(ctx.clone(), |value: String| dom::parse_url(&value))
                .map_err(|_| "Could not install the plugin bridge.".to_owned())?,
        )
        .map_err(|_| "Could not install the plugin bridge.".to_owned())?;
    globals
        .set(
            "__senzo_text_encode",
            Function::new(ctx.clone(), |value: String| hex::encode(value.as_bytes()))
                .map_err(|_| "Could not install the plugin bridge.".to_owned())?,
        )
        .map_err(|_| "Could not install the plugin bridge.".to_owned())?;
    globals
        .set(
            "__senzo_text_decode",
            Function::new(ctx.clone(), |value: String| {
                String::from_utf8_lossy(&hex::decode(value).unwrap_or_default()).into_owned()
            })
            .map_err(|_| "Could not install the plugin bridge.".to_owned())?,
        )
        .map_err(|_| "Could not install the plugin bridge.".to_owned())?;

    // Crypto bridges.
    globals
        .set(
            "__crypto_digest_hex",
            Function::new(ctx.clone(), |hash: String, data: String| -> rquickjs::Result<String> {
                let bytes = hex::decode(data).unwrap_or_default();
                crypto::digest(&hash, &bytes)
                    .map(hex::encode)
                    .map_err(|message| rquickjs::Error::new_from_js_message("plugin", "crypto", message))
            })
            .map_err(|_| "Could not install the plugin bridge.".to_owned())?,
        )
        .map_err(|_| "Could not install the plugin bridge.".to_owned())?;
    globals
        .set(
            "__crypto_hmac_hex",
            Function::new(
                ctx.clone(),
                |hash: String, key: String, data: String| -> rquickjs::Result<String> {
                    let key = hex::decode(key).unwrap_or_default();
                    let data = hex::decode(data).unwrap_or_default();
                    crypto::hmac(&hash, &key, &data)
                        .map(hex::encode)
                        .map_err(|message| rquickjs::Error::new_from_js_message("plugin", "crypto", message))
                },
            )
            .map_err(|_| "Could not install the plugin bridge.".to_owned())?,
        )
        .map_err(|_| "Could not install the plugin bridge.".to_owned())?;
    globals
        .set(
            "__crypto_pbkdf2_hex",
            Function::new(
                ctx.clone(),
                |password: String, salt: String, iterations: u32, bits: u32, hash: String| -> rquickjs::Result<String> {
                    let password = hex::decode(password).unwrap_or_default();
                    let salt = hex::decode(salt).unwrap_or_default();
                    crypto::pbkdf2(&password, &salt, iterations, bits, &hash)
                        .map(hex::encode)
                        .map_err(|message| rquickjs::Error::new_from_js_message("plugin", "crypto", message))
                },
            )
            .map_err(|_| "Could not install the plugin bridge.".to_owned())?,
        )
        .map_err(|_| "Could not install the plugin bridge.".to_owned())?;
    globals
        .set(
            "__crypto_aes_encrypt_hex",
            Function::new(
                ctx.clone(),
                |mode: String, key: String, iv: String, data: String| -> rquickjs::Result<String> {
                    let key = hex::decode(key).unwrap_or_default();
                    let iv = hex::decode(iv).unwrap_or_default();
                    let data = hex::decode(data).unwrap_or_default();
                    crypto::aes(true, &mode, &key, &iv, &data)
                        .map(hex::encode)
                        .map_err(|message| rquickjs::Error::new_from_js_message("plugin", "crypto", message))
                },
            )
            .map_err(|_| "Could not install the plugin bridge.".to_owned())?,
        )
        .map_err(|_| "Could not install the plugin bridge.".to_owned())?;
    globals
        .set(
            "__crypto_aes_decrypt_hex",
            Function::new(
                ctx.clone(),
                |mode: String, key: String, iv: String, data: String| -> rquickjs::Result<String> {
                    let key = hex::decode(key).unwrap_or_default();
                    let iv = hex::decode(iv).unwrap_or_default();
                    let data = hex::decode(data).unwrap_or_default();
                    crypto::aes(false, &mode, &key, &iv, &data)
                        .map(hex::encode)
                        .map_err(|message| rquickjs::Error::new_from_js_message("plugin", "crypto", message))
                },
            )
            .map_err(|_| "Could not install the plugin bridge.".to_owned())?,
        )
        .map_err(|_| "Could not install the plugin bridge.".to_owned())?;
    globals
        .set(
            "__crypto_random_hex",
            Function::new(ctx.clone(), |length: usize| hex::encode(crypto::random_bytes(length)))
                .map_err(|_| "Could not install the plugin bridge.".to_owned())?,
        )
        .map_err(|_| "Could not install the plugin bridge.".to_owned())?;

    install_dom(ctx, dom_store)?;

    ctx.eval::<(), _>(PRELUDE)
        .map_err(|_| "Could not initialize the provider module scope.".to_owned())?;
    ctx.eval::<(), _>(include_str!("polyfills.js"))
        .map_err(|error| format!("Could not initialize the provider compatibility layer: {error}"))?;
    ctx.eval::<(), _>(include_str!("polyfills_crypto.js"))
        .map_err(|error| format!("Could not initialize the provider crypto layer: {error}"))?;
    ctx.eval::<(), _>(include_str!("polyfills_dom.js"))
        .map_err(|error| format!("Could not initialize the provider DOM layer: {error}"))?;
    if execute_script {
        ctx.eval::<(), _>(script.as_bytes())
            .map_err(|error| format!("Provider script failed to load: {error}"))?;
    }
    Ok(())
}

fn install_dom<'js>(ctx: &Ctx<'js>, store: Rc<DomStore>) -> Result<(), String> {
    let globals = ctx.globals();
    macro_rules! install {
        ($name:literal, $closure:expr) => {
            globals
                .set(
                    $name,
                    Function::new(ctx.clone(), $closure)
                        .map_err(|_| "Could not install the DOM bridge.".to_owned())?,
                )
                .map_err(|_| "Could not install the DOM bridge.".to_owned())?;
        };
    }
    let load_store = store.clone();
    install!("__cheerio_load", move |html: String| load_store.load(&html));
    let select_store = store.clone();
    install!("__cheerio_select", move |document: usize, selector: String| {
        select_store.select(document, &selector)
    });
    let find_store = store.clone();
    install!(
        "__cheerio_find",
        move |document: usize, element: usize, selector: String| {
            find_store.find(document, element, &selector)
        }
    );
    let children_store = store.clone();
    install!(
        "__cheerio_children",
        move |document: usize, element: usize, selector: String| {
            children_store.children(document, element, &selector)
        }
    );
    let text_store = store.clone();
    install!("__cheerio_text", move |document: usize, element: usize| {
        text_store.text(document, element)
    });
    let inner_store = store.clone();
    install!(
        "__cheerio_inner_html",
        move |document: usize, element: usize| inner_store.inner_html(document, element)
    );
    let outer_store = store.clone();
    install!("__cheerio_html", move |document: usize, element: usize| {
        outer_store.outer_html(document, element)
    });
    let attr_store = store.clone();
    install!(
        "__cheerio_attr",
        move |document: usize, element: usize, name: String| {
            attr_store
                .attr(document, element, &name)
                .unwrap_or_else(|| "__UNDEFINED__".to_owned())
        }
    );
    let next_store = store.clone();
    install!("__cheerio_next", move |document: usize, element: usize| {
        next_store
            .next(document, element)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "__NONE__".to_owned())
    });
    let prev_store = store.clone();
    install!("__cheerio_prev", move |document: usize, element: usize| {
        prev_store
            .prev(document, element)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "__NONE__".to_owned())
    });
    let parent_store = store.clone();
    install!("__cheerio_parent", move |document: usize, element: usize| {
        parent_store
            .parent(document, element)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "__NONE__".to_owned())
    });
    let is_store = store.clone();
    install!(
        "__cheerio_is",
        move |document: usize, element: usize, selector: String| {
            is_store.is(document, element, &selector)
        }
    );
    let closest_store = store;
    install!(
        "__cheerio_closest",
        move |document: usize, element: usize, selector: String| {
            closest_store
                .closest(document, element, &selector)
                .map(|value| value.to_string())
                .unwrap_or_else(|| "__NONE__".to_owned())
        }
    );
    Ok(())
}

fn parse_streams(provider_key: &str, provider_name: &str, raw: &str) -> Result<Vec<PluginStream>, String> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|_| "The provider returned malformed JSON.".to_owned())?;
    let items = value
        .as_array()
        .or_else(|| value.get("streams").and_then(Value::as_array))
        .ok_or_else(|| "The provider did not return a stream list.".to_owned())?;
    let mut streams = Vec::new();
    for item in items.iter().take(MAX_RESULT_STREAMS * 2) {
        let Some(object) = item.as_object() else {
            continue;
        };
        let name = string_field(object, &["name", "provider", "source"]);
        let title = string_field(object, &["title", "description", "name"]);
        let url = string_field(object, &["url", "externalUrl", "link"])
            .or_else(|| first_magnet(object));
        let mut info_hash = string_field(object, &["infoHash", "info_hash", "hash"]);
        if info_hash.is_none() {
            info_hash = url.as_deref().and_then(magnet_hash);
        }
        let has_valid_hash = info_hash
            .as_deref()
            .is_some_and(is_info_hash);
        let has_valid_url = url.as_deref().is_some_and(|url| {
            url.starts_with("http://")
                || url.starts_with("https://")
                || (url.starts_with("magnet:") && has_valid_hash)
        });
        if !has_valid_url && !has_valid_hash {
            continue;
        }
        if !has_valid_hash {
            info_hash = None;
        }
        let file_idx = object
            .get("fileIdx")
            .or_else(|| object.get("file_idx"))
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok());
        let headers = header_map(object);
        let subtitles = subtitle_list(object);
        let quality = string_field(object, &["quality", "resolution"]);
        let display_name = match (quality, name) {
            (Some(quality), Some(name)) if !name.contains(&quality) => format!("{name} · {quality}"),
            (_, Some(name)) => name,
            (Some(quality), None) => quality,
            _ => "Plugin source".to_owned(),
        };
        let description = title.unwrap_or_else(|| display_name.clone());
        let seeders = number_field(object, &["seeders", "seeds", "peers"])
            .or_else(|| seeders_from_text(&description))
            .or_else(|| seeders_from_text(&display_name));
        if streams.iter().any(|stream: &PluginStream| {
            (stream.url.is_some() && stream.url == url)
                || (stream.info_hash.is_some() && stream.info_hash == info_hash)
        }) {
            continue;
        }
        streams.push(PluginStream {
            provider_key: provider_key.to_owned(),
            provider_name: provider_name.to_owned(),
            name: truncate(&display_name, 160),
            description: truncate(&description, 500),
            url: url.filter(|value| value.starts_with("http") || value.starts_with("magnet:")),
            info_hash,
            file_idx,
            headers,
            subtitles,
            seeders,
        });
        if streams.len() >= MAX_RESULT_STREAMS {
            break;
        }
    }
    Ok(streams)
}

fn first_magnet(object: &serde_json::Map<String, Value>) -> Option<String> {
    object
        .get("magnet")
        .and_then(Value::as_str)
        .filter(|value| value.starts_with("magnet:"))
        .map(str::to_owned)
        .or_else(|| {
            object
                .get("sources")
                .and_then(Value::as_array)
                .and_then(|sources| {
                    sources
                        .iter()
                        .filter_map(Value::as_str)
                        .find(|value| value.starts_with("magnet:"))
                        .map(str::to_owned)
                })
        })
}

fn number_field(object: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<u32> {
    keys.iter().find_map(|key| {
        let value = object.get(*key)?;
        value
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .or_else(|| value.as_str().and_then(|value| value.trim().parse().ok()))
    })
}

fn seeders_from_text(value: &str) -> Option<u32> {
    for marker in ["👤", "Seeders:", "Seeders", "Seeds:", "Seeds"] {
        let mut search = 0usize;
        while let Some(relative) = value[search..].find(marker) {
            let start = search + relative + marker.len();
            let tail = value[start..].trim_start_matches(|character: char| {
                character.is_whitespace() || matches!(character, ':' | '=')
            });
            let digits = tail
                .chars()
                .take_while(|character| character.is_ascii_digit())
                .collect::<String>();
            if let Ok(seeders) = digits.parse() {
                return Some(seeders);
            }
            search = start;
        }
    }
    None
}

fn is_info_hash(hash: &str) -> bool {
    hash.len() == 40 && hash.chars().all(|character| character.is_ascii_hexdigit())
}

fn magnet_hash(value: &str) -> Option<String> {
    let query = value.strip_prefix("magnet:?")?;
    url::form_urlencoded::parse(query.as_bytes())
        .filter(|(key, _)| key == "xt")
        .find_map(|(_, value)| value.strip_prefix("urn:btih:").map(str::to_owned))
}

fn string_field(object: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    })
}

fn header_map(object: &serde_json::Map<String, Value>) -> Vec<(String, String)> {
    let hints = object.get("behaviorHints");
    let source = object
        .get("headers")
        .and_then(Value::as_object)
        .or_else(|| hints.and_then(|value| value.get("proxyHeaders")).and_then(|value| value.get("request")).and_then(Value::as_object))
        .or_else(|| hints.and_then(|value| value.get("headers")).and_then(Value::as_object))
        .or_else(|| hints.and_then(|value| value.get("proxyHeaders")).and_then(Value::as_object));
    let Some(source) = source else {
        return Vec::new();
    };
    source
        .iter()
        .filter_map(|(name, value)| {
            let name = name.trim().to_ascii_lowercase();
            if name.is_empty()
                || name.len() > 64
                || !name
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-' || character == '_')
                || matches!(name.as_str(), "host" | "content-length" | "connection")
            {
                return None;
            }
            let value = value.as_str()?.trim();
            if value.is_empty()
                || value.len() > 2048
                || value.contains('\r')
                || value.contains('\n')
            {
                return None;
            }
            Some((name, value.to_owned()))
        })
        .take(24)
        .collect()
}

fn subtitle_list(object: &serde_json::Map<String, Value>) -> Vec<String> {
    let Some(items) = object.get("subtitles").and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .take(16)
        .filter_map(|item| {
            let url = item
                .as_str()
                .or_else(|| item.get("url").and_then(Value::as_str))?;
            if url.starts_with("https://") || url.starts_with("http://") {
                Some(url.to_owned())
            } else {
                None
            }
        })
        .collect()
}

fn redact_log(value: &str) -> String {
    let mut output = value.to_owned();
    for key in ["api_key", "apikey", "access_token", "token"] {
        let mut search = 0usize;
        while let Some(relative) = output[search..].find(key) {
            let start = search + relative;
            let Some(equals) = output[start..].find('=') else { break; };
            let value_start = start + equals + 1;
            let value_end = output[value_start..]
                .find(['&', ' ', '"'])
                .map(|offset| value_start + offset)
                .unwrap_or(output.len());
            if value_end > value_start {
                output.replace_range(value_start..value_end, "[redacted]");
            }
            search = value_start + "[redacted]".len();
            if search >= output.len() { break; }
        }
    }
    output
}

fn truncate(value: &str, limit: usize) -> String {
    let mut output = String::with_capacity(value.len().min(limit));
    for character in value.chars() {
        if output.len() + character.len_utf8() > limit {
            break;
        }
        if character.is_control() && character != '\n' && character != '\t' {
            continue;
        }
        output.push(character);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_http_and_torrent_results() {
        let raw = r#"[
          {"name":"1080p","title":"Movie","url":"https://example.com/video.mp4","quality":"1080p","seeders":12,"headers":{"Referer":"https://example.com/"}},
          {"name":"Torrent","infoHash":"0123456789abcdef0123456789abcdef01234567","fileIdx":1,"url":""}
        ]"#;
        let streams = parse_streams("provider", "Provider", raw).unwrap();
        assert_eq!(streams.len(), 2);
        assert_eq!(streams[0].headers.len(), 1);
        assert_eq!(streams[0].seeders, Some(12));
        assert_eq!(streams[1].file_idx, Some(1));
    }

    #[test]
    fn rejects_invalid_results() {
        let streams = parse_streams("provider", "Provider", r#"[{"name":"bad","url":"javascript:alert(1)"}]"#).unwrap();
        assert!(streams.is_empty());
    }

    #[test]
    fn crypto_js_and_webcrypto_are_available_to_scripts() {
        let script = r#"
const CryptoJS = require("crypto-js");
async function getStreams() {
  const digest = CryptoJS.MD5("abc").toString(CryptoJS.enc.Hex);
  const subtle = await crypto.subtle.digest("SHA-256", new TextEncoder().encode("abc"));
  return [{ name: digest, url: "https://example.com/" + Buffer.from(subtle).toString("hex") }];
}
module.exports = { getStreams };
"#;
        let outcome = execute(
            "test",
            "Test",
            script,
            "{}",
            &ProviderRequest {
                tmdb_id: "0".to_owned(),
                media_type: "movie".to_owned(),
                season: None,
                episode: None,
            },
        );
        assert_eq!(outcome.error, None);
        assert_eq!(outcome.streams.len(), 1);
        assert_eq!(outcome.streams[0].name, "900150983cd24fb0d6963f7d28e17f72");
        assert!(outcome.streams[0]
            .url
            .as_deref()
            .is_some_and(|url| url.ends_with(
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
            )));
    }

    #[test]
    fn validates_a_simple_script() {
        validate(
            "test",
            "async function getStreams() { return []; } module.exports = { getStreams };",
            "{}",
        )
        .unwrap();
    }
}
