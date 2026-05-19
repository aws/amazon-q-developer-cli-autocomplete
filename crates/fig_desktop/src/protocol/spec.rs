//! Local-only spec:// protocol handler.
//!
//! Serves Fig autocomplete specs from a directory on disk instead of fetching
//! them from a CDN. The directory is resolved in this order:
//!
//!   1. `$PRECOG_SPECS_DIR` env var (highest priority — for development)
//!   2. `<exe-dir>/../Resources/autocomplete-specs/build` (macOS app bundle)
//!   3. `<exe-dir>/../share/precog/autocomplete-specs/build` (Linux install)
//!   4. `<repo>/packages/autocomplete-specs/build` (cargo run from source)
//!
//! No network calls. No AWS endpoints. Specs are vendored at
//! `packages/autocomplete-specs/`.

use std::borrow::Cow;
use std::path::{
    Path,
    PathBuf,
};
use std::sync::Arc;

use anyhow::Result;
use fig_os_shim::Context;
use serde::{
    Deserialize,
    Serialize,
};
use tokio::sync::Mutex;
use tracing::{
    debug,
    warn,
};
use wry::http::header::CONTENT_TYPE;
use wry::http::{
    HeaderValue,
    Request,
    Response,
    StatusCode,
};

use crate::webview::WindowId;

const APPLICATION_JAVASCRIPT: HeaderValue = HeaderValue::from_static("application/javascript");
const APPLICATION_JSON: HeaderValue = HeaderValue::from_static("application/json");
const IMAGE_PNG: HeaderValue = HeaderValue::from_static("image/png");

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpecIndex {
    completions: Vec<String>,
    diff_versioned_completions: Vec<String>,
}

static SPECS_DIR: tokio::sync::OnceCell<Option<PathBuf>> = tokio::sync::OnceCell::const_new();
static INDEX_CACHE: Mutex<Option<SpecIndex>> = Mutex::const_new(None);

pub async fn clear_index_cache() {
    *INDEX_CACHE.lock().await = None;
}

fn candidate_dirs() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(p) = std::env::var("PRECOG_SPECS_DIR") {
        paths.push(PathBuf::from(p));
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            // macOS bundle: <App>.app/Contents/MacOS/<exe> -> Contents/Resources/autocomplete-specs/build
            paths.push(exe_dir.join("../Resources/autocomplete-specs/build"));
            // Linux: <prefix>/bin/<exe> -> <prefix>/share/precog/autocomplete-specs/build
            paths.push(exe_dir.join("../share/precog/autocomplete-specs/build"));
        }
    }

    // cargo-run-from-source fallback: walk up from CWD looking for the workspace root
    if let Ok(mut cwd) = std::env::current_dir() {
        for _ in 0..6 {
            let candidate = cwd.join("packages/autocomplete-specs/build");
            paths.push(candidate);
            if !cwd.pop() {
                break;
            }
        }
    }

    paths
}

async fn resolve_specs_dir() -> Option<PathBuf> {
    SPECS_DIR
        .get_or_init(|| async {
            for p in candidate_dirs() {
                if tokio::fs::try_exists(&p).await.unwrap_or(false) {
                    debug!("Resolved specs dir: {}", p.display());
                    return Some(p);
                }
            }
            warn!(
                "No autocomplete specs directory found. Set PRECOG_SPECS_DIR or run \
                 `pnpm -C packages/autocomplete-specs build`."
            );
            None
        })
        .await
        .clone()
}

async fn read_index(dir: &Path) -> Result<SpecIndex> {
    let manifest = dir.join("index.json");
    if let Ok(bytes) = tokio::fs::read(&manifest).await {
        if let Ok(idx) = serde_json::from_slice::<SpecIndex>(&bytes) {
            return Ok(idx);
        }
    }

    // Fallback: build the index by walking the directory.
    let mut completions = Vec::new();
    let mut diff_versioned = Vec::new();
    let mut entries = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with('.') || name.starts_with('@') {
            continue;
        }
        let file_type = entry.file_type().await?;
        if file_type.is_dir() {
            diff_versioned.push(name.to_string());
        } else if file_type.is_file() {
            if let Some(stem) = name.strip_suffix(".js") {
                completions.push(stem.to_string());
            }
        }
    }
    completions.sort();
    diff_versioned.sort();
    Ok(SpecIndex {
        completions,
        diff_versioned_completions: diff_versioned,
    })
}

async fn cached_index(dir: &Path) -> SpecIndex {
    let mut cache = INDEX_CACHE.lock().await;
    if cache.is_none() {
        match read_index(dir).await {
            Ok(idx) => *cache = Some(idx),
            Err(err) => {
                warn!(%err, "Failed to read spec index");
                *cache = Some(SpecIndex::default());
            },
        }
    }
    cache.clone().unwrap_or_default()
}

fn res_404() -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header(CONTENT_TYPE, "text/plain")
        .body(b"Not Found".as_ref().into())
        .unwrap()
}

fn res_ok(bytes: Vec<u8>, content_type: HeaderValue) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, content_type)
        .body(bytes.into())
        .unwrap()
}

fn content_type_for(path: &Path) -> HeaderValue {
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => APPLICATION_JSON,
        Some("png") => IMAGE_PNG,
        _ => APPLICATION_JAVASCRIPT,
    }
}

// handle `spec://localhost/spec.js`
pub async fn handle(
    _ctx: Arc<Context>,
    request: Request<Vec<u8>>,
    _: WindowId,
) -> anyhow::Result<Response<Cow<'static, [u8]>>> {
    let Some(specs_dir) = resolve_specs_dir().await else {
        return Ok(res_404());
    };

    let path = request.uri().path();
    let rel = path.strip_prefix('/').unwrap_or(path);

    if rel == "index.json" {
        let idx = cached_index(&specs_dir).await;
        return Ok(res_ok(serde_json::to_vec(&idx)?, APPLICATION_JSON));
    }

    // Reject path traversal.
    if rel.contains("..") {
        return Ok(res_404());
    }

    // Try the requested file directly, then `<rel>/index.js` for diff-versioned specs.
    let primary = specs_dir.join(rel);
    let candidate = if tokio::fs::try_exists(&primary).await.unwrap_or(false) {
        primary
    } else {
        let with_index = specs_dir.join(rel.trim_end_matches(".js")).join("index.js");
        if tokio::fs::try_exists(&with_index).await.unwrap_or(false) {
            with_index
        } else {
            return Ok(res_404());
        }
    };

    match tokio::fs::read(&candidate).await {
        Ok(bytes) => Ok(res_ok(bytes, content_type_for(&candidate))),
        Err(err) => {
            warn!(?candidate, %err, "Failed to read spec file");
            Ok(res_404())
        },
    }
}
