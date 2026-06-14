//! Embedded operator UI (the analytics console), served read-only at `/ui/*`.
//!
//! Appliance posture (candidate-ADR `05-ui-track`): the whole console is
//! compiled INTO the `meridiand` musl binary via `include_bytes!` — no
//! filesystem reads at runtime, no `tower-http` fs feature, no new Cargo
//! dependency, no build toolchain, no CDN/egress. The assets are vanilla ES
//! modules + plain CSS + hand-rolled `<canvas>` charts (zero third-party JS).
//!
//! The static shell is GET-only and unauthenticated (it inherits the existing
//! "enable auth + TLS before exposing" gate, operator-manual.md); the JS
//! attaches an operator-supplied bearer ONLY to guarded endpoints. These routes
//! are merged OUTSIDE the `/v1/*` guardrail + body-limit layer so static asset
//! delivery is never rate-limited or shed — the assets are immutable bytes in
//! the binary, not a request-amplification surface.

use axum::extract::Path;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

/// One embedded asset: its MIME type and its bytes baked into the binary.
struct Asset {
    mime: &'static str,
    bytes: &'static [u8],
}

const HTML: &str = "text/html; charset=utf-8";
const JS: &str = "text/javascript; charset=utf-8";
const CSS: &str = "text/css; charset=utf-8";

/// Map a request path (already stripped of the `/ui/` prefix) to an embedded
/// asset. Every panel module is listed up front so the parallel panel authors'
/// files resolve the moment they land. Unknown paths return `None` → 404.
///
/// Paths are matched verbatim (no `..` traversal is possible — there is no
/// filesystem; only these exact keys exist).
fn lookup(path: &str) -> Option<Asset> {
    let asset = match path {
        // The shell. Both the bare prefix and `index.html` resolve here.
        "" | "index.html" => Asset {
            mime: HTML,
            bytes: include_bytes!("../assets/ui/index.html"),
        },
        "app.js" => Asset {
            mime: JS,
            bytes: include_bytes!("../assets/ui/app.js"),
        },
        "style.css" => Asset {
            mime: CSS,
            bytes: include_bytes!("../assets/ui/style.css"),
        },
        "panels/lanes.js" => Asset {
            mime: JS,
            bytes: include_bytes!("../assets/ui/panels/lanes.js"),
        },
        "panels/trends.js" => Asset {
            mime: JS,
            bytes: include_bytes!("../assets/ui/panels/trends.js"),
        },
        "panels/geo.js" => Asset {
            mime: JS,
            bytes: include_bytes!("../assets/ui/panels/geo.js"),
        },
        "panels/search.js" => Asset {
            mime: JS,
            bytes: include_bytes!("../assets/ui/panels/search.js"),
        },
        "panels/ope.js" => Asset {
            mime: JS,
            bytes: include_bytes!("../assets/ui/panels/ope.js"),
        },
        "panels/metrics.js" => Asset {
            mime: JS,
            bytes: include_bytes!("../assets/ui/panels/metrics.js"),
        },
        "panels/deploy.js" => Asset {
            mime: JS,
            bytes: include_bytes!("../assets/ui/panels/deploy.js"),
        },
        _ => return None,
    };
    Some(asset)
}

/// Build the response headers for an asset: content type + a conservative
/// cache policy. The assets are immutable for the lifetime of a binary build,
/// but we deliberately avoid an aggressive `immutable`/long max-age so an
/// in-place binary upgrade is reflected on the operator's next load.
fn asset_headers(mime: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, max-age=0"),
    );
    headers
}

/// `GET /ui` and `GET /ui/` → the shell (`index.html`).
pub async fn index() -> Response {
    // `index.html` is always present (it is `include_bytes!`'d above), so this
    // lookup cannot miss — but stay total rather than unwrap.
    match lookup("index.html") {
        Some(asset) => (asset_headers(asset.mime), asset.bytes).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `GET /ui/{*path}` → the named embedded asset, or 404 for anything not in the
/// table. `&'static [u8]` already implements `IntoResponse` in axum, so the
/// body is served without copying.
pub async fn asset(Path(path): Path<String>) -> Response {
    match lookup(path.as_str()) {
        Some(asset) => (asset_headers(asset.mime), asset.bytes).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
