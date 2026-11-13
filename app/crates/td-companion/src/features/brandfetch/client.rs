//! The Brand API call behind a pick: find the brand with the user's API key, choose its best
//! logo, download that file and draw it as a 64x64 icon. Search never comes here; the UI calls
//! the Brand Search API itself. The Companion never builds a Logo API address
//! (`cdn.brandfetch.io/<domain>?c=`); it downloads only the `src` a Brand API answer gives.
use super::{BrandfetchError, BrandfetchLogo, LogoTheme, LogoType};
use crate::features::assets::Fit;
use image::RgbaImage;
use serde::Deserialize;
use std::time::Duration;
use td_host::icon_pack::{self, SvgSource};

pub const API_BASE: &str = "https://api.brandfetch.io";
/// The hosts the Companion downloads a logo file from: only the `src` addresses the Brand API
/// answer gives, fetched unchanged and without the key. Brandfetch serves them from
/// `cdn.brandfetch.io` (older answers used `asset.brandfetch.io`) and says they work from a
/// server; fetching them doesn't count against the quota. A `src` anywhere else is skipped.
pub const FILE_ORIGINS: &[&str] = &["https://cdn.brandfetch.io", "https://asset.brandfetch.io"];
const FILE_BYTES_MAX: u64 = 4 << 20;
const TIMEOUT: Duration = Duration::from_secs(20);
/// Logo files one pick downloads at most before it gives up.
const FILE_TRIES: usize = 4;

/// The user's Brand API key. It prints as `[redacted]`, and leaves this process only as the
/// `Authorization: Bearer` header of a Brand API call.
#[derive(Clone)]
pub struct ApiKey(zeroize::Zeroizing<String>);
impl ApiKey {
    pub fn new(value: String) -> Self {
        Self(zeroize::Zeroizing::new(value))
    }
    pub(super) fn expose(&self) -> &str {
        &self.0
    }
    /// Removes the key from text bound for an error, should a transport error ever quote it.
    pub fn scrub(&self, text: String) -> String {
        if self.0.is_empty() {
            text
        } else {
            text.replace(self.expose(), "[redacted]")
        }
    }
}
impl From<&str> for ApiKey {
    fn from(value: &str) -> Self {
        Self::new(value.to_owned())
    }
}
impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}
impl std::fmt::Display for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}

pub struct Response {
    pub status: u16,
    /// Brandfetch's `x-bf-error` reason, when it sent one.
    pub reason: Option<String>,
    pub body: Vec<u8>,
}
/// One GET. Implementations bound the time and the bytes read and never follow a redirect,
/// so a bearer key can't travel to another host.
pub trait Http: Send + Sync {
    fn get(&self, url: &str, bearer: Option<&ApiKey>) -> Result<Response, String>;
}
/// Real HTTP. It refuses anything but HTTPS, except for a test's local stand-in.
pub struct UreqHttp {
    https_only: bool,
}
impl UreqHttp {
    pub fn https() -> Self {
        Self { https_only: true }
    }
    /// Plain HTTP as well, for tests against a server on loopback.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn local() -> Self {
        Self { https_only: false }
    }
}
impl Http for UreqHttp {
    fn get(&self, url: &str, bearer: Option<&ApiKey>) -> Result<Response, String> {
        let mut request = ureq::get(url);
        if let Some(key) = bearer {
            request = request.header("Authorization", &format!("Bearer {}", key.expose()));
        }
        let mut response = request
            .config()
            .https_only(self.https_only)
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            .max_redirects(0)
            .build()
            .call()
            .map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        let reason = response
            .headers()
            .get("x-bf-error")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let body = response
            .body_mut()
            .with_config()
            .limit(FILE_BYTES_MAX)
            .read_to_vec()
            .map_err(|e| e.to_string())?;
        Ok(Response {
            status,
            reason,
            body,
        })
    }
}

/// A logo file as Brandfetch served it.
#[derive(Clone)]
pub struct Original {
    pub bytes: Vec<u8>,
    pub format: FileFormat,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileFormat {
    Svg,
    Png,
    Webp,
    Jpeg,
}
impl FileFormat {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "svg" => Some(Self::Svg),
            "png" => Some(Self::Png),
            "webp" => Some(Self::Webp),
            "jpeg" | "jpg" => Some(Self::Jpeg),
            _ => None,
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Self::Svg => "svg",
            Self::Png => "png",
            Self::Webp => "webp",
            Self::Jpeg => "jpeg",
        }
    }
}

#[derive(Deserialize)]
struct Brand {
    #[serde(default)]
    logos: Vec<Logo>,
}
#[derive(Deserialize)]
struct Logo {
    #[serde(rename = "type")]
    kind: Option<String>,
    theme: Option<String>,
    #[serde(default)]
    formats: Vec<Format>,
}
#[derive(Deserialize)]
struct Format {
    src: String,
    format: String,
}

/// A domain such as `example.com` or a Brandfetch brand id such as `id_0dwKPKT`.
pub fn valid_brand(brand: &str) -> bool {
    (1..=253).contains(&brand.len())
        && !brand.starts_with(['.', '-'])
        && brand
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// Whether `url` is an HTTPS file on `origin`, with nothing that could point elsewhere.
fn on_origin(url: &str, origin: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| {
        u.origin().ascii_serialization() == origin
            && u.username().is_empty()
            && u.password().is_none()
    })
}

/// Calls the Brand API and downloads the best file of the requested logo type, or of the
/// next type when the brand has none. `files` lists the origins a logo file may come from.
/// Between downloads it stops once `cancelled` says so.
pub fn fetch(
    http: &dyn Http,
    base: &str,
    files: &[String],
    key: &ApiKey,
    logo: &BrandfetchLogo,
    cancelled: &dyn Fn() -> bool,
) -> Result<Original, BrandfetchError> {
    let brand = brand(http, base, key, &logo.brand)?;
    let candidates = choose(&brand, logo.logo_type, logo.theme, files);
    if candidates.is_empty() {
        return Err(BrandfetchError::NoLogo);
    }
    let mut last = None;
    for (src, format) in candidates.into_iter().take(FILE_TRIES) {
        if cancelled() {
            return Err(BrandfetchError::Unavailable(
                "the pick was cancelled".into(),
            ));
        }
        match http.get(&src, None) {
            Ok(file) if file.status == 200 && !file.body.is_empty() => {
                return Ok(Original {
                    bytes: file.body,
                    format,
                });
            }
            Ok(file) => last = Some(format!("the logo file answered {}", file.status)),
            Err(error) => last = Some(key.scrub(error)),
        }
    }
    Err(BrandfetchError::Unavailable(
        last.unwrap_or_else(|| "no logo file".into()),
    ))
}

/// One Brand API call. The key travels only as a Bearer header, only over HTTPS (or to a
/// loopback stand-in in tests), and never in the URL.
fn brand(http: &dyn Http, base: &str, key: &ApiKey, brand: &str) -> Result<Brand, BrandfetchError> {
    if !valid_brand(brand) {
        return Err(BrandfetchError::NotFound);
    }
    td_host::integrations::secrets::credential_transport(base)
        .map_err(|_| BrandfetchError::Unavailable("the Brand API address must use HTTPS".into()))?;
    let url = format!("{base}/v2/brands/{brand}");
    let response = http
        .get(&url, Some(key))
        .map_err(|e| BrandfetchError::Unavailable(key.scrub(e)))?;
    match (response.status, response.reason.as_deref()) {
        (200, _) => {}
        (401 | 402, _) => return Err(BrandfetchError::KeyRefused),
        (403, Some("no_api_credits")) | (429, _) => return Err(BrandfetchError::Quota),
        (403, _) => return Err(BrandfetchError::KeyRefused),
        (400 | 404, _) => return Err(BrandfetchError::NotFound),
        (status, _) => {
            return Err(BrandfetchError::Unavailable(format!(
                "Brandfetch answered {status}"
            )));
        }
    }
    serde_json::from_slice(&response.body)
        .map_err(|_| BrandfetchError::Unavailable("Brandfetch sent an unreadable answer".into()))
}

/// The brand whose Brand API lookups Brandfetch doesn't count against the quota.
const FREE_BRAND: &str = "brandfetch.com";
/// Checks the key with one Brand API call for Brandfetch's own brand.
pub fn check(http: &dyn Http, base: &str, key: &ApiKey) -> Result<(), BrandfetchError> {
    brand(http, base, key, FREE_BRAND).map(|_| ())
}

/// Every downloadable file, best first: the requested type, then the other types in the order
/// icon, symbol, logo, so a brand without an icon still gives its symbol or logo. Within a type,
/// logos go by theme preference, and within a logo SVG, PNG, WebP and JPEG.
fn choose(
    brand: &Brand,
    kind: LogoType,
    theme: LogoTheme,
    files: &[String],
) -> Vec<(String, FileFormat)> {
    // The Device's Tiles are dark, so `auto` prefers a light logo.
    let order: &[Option<&str>] = match theme {
        LogoTheme::Auto => &[Some("light"), None, Some("dark")],
        LogoTheme::Light => &[Some("light"), None],
        LogoTheme::Dark => &[Some("dark"), None],
    };
    let kinds: Vec<LogoType> = std::iter::once(kind)
        .chain(
            [LogoType::Icon, LogoType::Symbol, LogoType::Logo]
                .into_iter()
                .filter(|k| *k != kind),
        )
        .collect();
    let mut logos: Vec<(usize, usize, &Logo)> = brand
        .logos
        .iter()
        .filter_map(|l| {
            let kind_rank = kinds
                .iter()
                .position(|k| l.kind.as_deref() == Some(k.name()))?;
            let theme_rank = order.iter().position(|t| *t == l.theme.as_deref())?;
            Some((kind_rank, theme_rank, l))
        })
        .collect();
    logos.sort_by_key(|(kind_rank, theme_rank, _)| (*kind_rank, *theme_rank));
    let mut out = Vec::new();
    for (_, _, logo) in logos {
        let mut formats: Vec<(FileFormat, &str)> = logo
            .formats
            .iter()
            .filter(|f| files.iter().any(|origin| on_origin(&f.src, origin)))
            .filter_map(|f| FileFormat::parse(&f.format).map(|format| (format, f.src.as_str())))
            .collect();
        formats.sort_by_key(|(format, _)| *format);
        out.extend(formats.into_iter().map(|(f, src)| (src.to_owned(), f)));
    }
    out
}

/// Draws a logo file as a 64x64 icon with the pick's padding, background and tint.
pub fn draw(original: &Original, logo: &BrandfetchLogo, fit: Fit) -> Result<RgbaImage, String> {
    let side = icon_pack::SIDE;
    if logo.padding >= side / 2 {
        return Err("Invalid padding".into());
    }
    let cover = matches!(fit, Fit::Cover);
    let image = if original.format == FileFormat::Svg {
        // A third party's file, drawn by the rules for uploads: no DOCTYPE, ENTITY or
        // compression, and no linked or embedded image is ever read.
        let inner = side - 2 * logo.padding;
        icon_pack::render_svg_fit_from(&original.bytes, inner, inner, cover, SvgSource::Upload)?
    } else {
        let format = match original.format {
            FileFormat::Png => image::ImageFormat::Png,
            FileFormat::Webp => image::ImageFormat::WebP,
            _ => image::ImageFormat::Jpeg,
        };
        let mut reader =
            image::ImageReader::with_format(std::io::Cursor::new(&original.bytes), format);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(4096);
        limits.max_alloc = Some(64 << 20);
        reader.limits(limits);
        reader.decode().map_err(|e| e.to_string())?.into_rgba8()
    };
    // A file that draws nothing, such as an SVG whose only content is an image it may not
    // read, makes no icon.
    if image.pixels().all(|pixel| pixel.0[3] == 0) {
        return Err("the logo draws nothing".into());
    }
    let image = if cover && original.format != FileFormat::Svg {
        let (w, h) = image.dimensions();
        let s = w.min(h);
        image::imageops::crop_imm(&image, (w - s) / 2, (h - s) / 2, s, s).to_image()
    } else {
        image
    };
    Ok(icon_pack::fit_tile(
        &image,
        side,
        side,
        logo.padding,
        logo.background,
        logo.tint,
    ))
}

/// The Companion's answers in mock mode: a recorded Brand API answer shape for any brand and a
/// small SVG per brand, so the demo and e2e runs never reach Brandfetch.
pub struct FixtureHttp;
pub const FIXTURE_FILES: &str = "https://cdn.brandfetch.io/fixture";
impl Http for FixtureHttp {
    fn get(&self, url: &str, bearer: Option<&ApiKey>) -> Result<Response, String> {
        let ok = |body: Vec<u8>| Response {
            status: 200,
            reason: None,
            body,
        };
        if let Some(brand) = url.strip_prefix(&format!("{API_BASE}/v2/brands/")) {
            if bearer.is_none() {
                return Ok(Response {
                    status: 401,
                    reason: Some("authorization_required".into()),
                    body: vec![],
                });
            }
            if brand.starts_with("missing") {
                return Ok(Response {
                    status: 404,
                    reason: Some("brand_not_found".into()),
                    body: vec![],
                });
            }
            let body = include_str!("fixture-brand.json").replace("{brand}", brand);
            return Ok(ok(body.into_bytes()));
        }
        if let Some(file) = url.strip_prefix(&format!("{FIXTURE_FILES}/")) {
            let hue = file
                .bytes()
                .fold(7u32, |h, b| h.wrapping_mul(31) ^ u32::from(b))
                % 360;
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect x="4" y="4" width="56" height="56" rx="14" fill="hsl({hue},70%,55%)"/><circle cx="32" cy="32" r="12" fill="#fff"/></svg>"##
            );
            return Ok(ok(svg.into_bytes()));
        }
        Err("The mock Companion reaches no network.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn brand(json: &str) -> Brand {
        serde_json::from_str(json).unwrap()
    }
    fn files() -> Vec<String> {
        FILE_ORIGINS.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn prefers_the_theme_then_svg_png_webp_jpeg_and_skips_other_hosts() {
        let b = brand(
            r#"{"logos":[
              {"type":"icon","theme":"dark","formats":[{"src":"https://asset.brandfetch.io/d.svg","format":"svg"}]},
              {"type":"icon","theme":"light","formats":[
                {"src":"https://asset.brandfetch.io/l.jpeg","format":"jpeg"},
                {"src":"https://elsewhere.example/l.svg","format":"svg"},
                {"src":"http://cdn.brandfetch.io/l.svg","format":"svg"},
                {"src":"https://asset.brandfetch.io/l.webp","format":"webp"},
                {"src":"https://cdn.brandfetch.io/l.png?c=test-client-id","format":"png"}]},
              {"type":"logo","theme":"light","formats":[{"src":"https://asset.brandfetch.io/x.svg","format":"svg"}]}
            ]}"#,
        );
        let srcs: Vec<_> = choose(&b, LogoType::Icon, LogoTheme::Auto, &files())
            .into_iter()
            .map(|(src, _)| src)
            .collect();
        assert_eq!(
            srcs,
            [
                "https://cdn.brandfetch.io/l.png?c=test-client-id",
                "https://asset.brandfetch.io/l.webp",
                "https://asset.brandfetch.io/l.jpeg",
                "https://asset.brandfetch.io/d.svg",
                // Then the other logo types, should every icon fail.
                "https://asset.brandfetch.io/x.svg",
            ]
        );
        let dark: Vec<_> = choose(&b, LogoType::Icon, LogoTheme::Dark, &files());
        assert_eq!(dark[0].0, "https://asset.brandfetch.io/d.svg");
    }

    #[test]
    fn a_brand_without_an_icon_falls_back_to_its_symbol_then_its_logo() {
        let b = brand(
            r#"{"logos":[
              {"type":"logo","theme":"light","formats":[{"src":"https://asset.brandfetch.io/logo.svg","format":"svg"}]},
              {"type":"symbol","theme":"light","formats":[{"src":"https://asset.brandfetch.io/symbol.png","format":"png"}]}
            ]}"#,
        );
        let srcs = |kind| -> Vec<String> {
            choose(&b, kind, LogoTheme::Auto, &files())
                .into_iter()
                .map(|(src, _)| src)
                .collect()
        };
        assert_eq!(
            srcs(LogoType::Icon),
            [
                "https://asset.brandfetch.io/symbol.png",
                "https://asset.brandfetch.io/logo.svg"
            ]
        );
        assert_eq!(
            srcs(LogoType::Logo)[0],
            "https://asset.brandfetch.io/logo.svg"
        );
        let none = brand(r#"{"logos":[]}"#);
        assert!(choose(&none, LogoType::Icon, LogoTheme::Auto, &files()).is_empty());
    }

    /// Answers the Brand API with `files` icon files on the CDN, then fails every download.
    struct Flaky {
        files: usize,
        downloads: std::sync::Mutex<usize>,
    }
    impl Http for Flaky {
        fn get(&self, url: &str, _: Option<&ApiKey>) -> Result<Response, String> {
            if url.contains("/v2/brands/") {
                let formats: Vec<String> = (0..self.files)
                    .map(|n| {
                        format!(r#"{{"src":"https://cdn.brandfetch.io/{n}.png","format":"png"}}"#)
                    })
                    .collect();
                let body = format!(
                    r#"{{"logos":[{{"type":"icon","theme":null,"formats":[{}]}}]}}"#,
                    formats.join(",")
                );
                return Ok(Response {
                    status: 200,
                    reason: None,
                    body: body.into_bytes(),
                });
            }
            *self.downloads.lock().unwrap() += 1;
            Err("offline".into())
        }
    }

    #[test]
    fn a_pick_tries_a_few_files_and_stops_when_cancelled() {
        let key = ApiKey::new("test-api-key".into());
        let logo = BrandfetchLogo {
            brand: "example.com".into(),
            logo_type: LogoType::Icon,
            theme: LogoTheme::Auto,
            padding: 0,
            background: None,
            tint: None,
        };
        let flaky = Flaky {
            files: 20,
            downloads: Default::default(),
        };
        let error = fetch(&flaky, API_BASE, &files(), &key, &logo, &|| false)
            .err()
            .unwrap();
        assert!(matches!(error, BrandfetchError::Unavailable(_)));
        assert_eq!(*flaky.downloads.lock().unwrap(), FILE_TRIES);

        let stopped = Flaky {
            files: 20,
            downloads: Default::default(),
        };
        let asked = std::cell::Cell::new(0);
        let cancelled = || {
            asked.set(asked.get() + 1);
            asked.get() > 1
        };
        assert!(fetch(&stopped, API_BASE, &files(), &key, &logo, &cancelled).is_err());
        assert_eq!(
            *stopped.downloads.lock().unwrap(),
            1,
            "one download, then the cancel"
        );
    }

    /// Records each call and answers 200 with no logos.
    #[derive(Default)]
    struct Spy(std::sync::Mutex<Vec<(String, bool)>>);
    impl Http for Spy {
        fn get(&self, url: &str, bearer: Option<&ApiKey>) -> Result<Response, String> {
            self.0.lock().unwrap().push((url.into(), bearer.is_some()));
            Ok(Response {
                status: 200,
                reason: None,
                body: br#"{"logos":[]}"#.to_vec(),
            })
        }
    }

    #[test]
    fn the_key_prints_as_redacted_and_is_scrubbed_from_errors() {
        let key = ApiKey::new("test-api-key".into());
        assert_eq!(format!("{key:?}"), "[redacted]");
        assert_eq!(format!("{key}"), "[redacted]");
        assert_eq!(format!("{:?}", Some(key.clone())), "Some([redacted])");
        assert_eq!(
            key.scrub("refused test-api-key at the door".into()),
            "refused [redacted] at the door"
        );
    }

    #[test]
    fn the_key_goes_only_over_https_and_never_in_a_url() {
        let key = ApiKey::new("test-api-key".into());
        let spy = Spy::default();
        let logo = BrandfetchLogo {
            brand: "example.com".into(),
            logo_type: LogoType::Icon,
            theme: LogoTheme::Auto,
            padding: 0,
            background: None,
            tint: None,
        };
        for base in [
            "http://api.brandfetch.io",
            "http://brandfetch.example",
            "ftp://x",
        ] {
            let error = fetch(&spy, base, &files(), &key, &logo, &|| false)
                .err()
                .unwrap();
            assert!(matches!(error, BrandfetchError::Unavailable(_)), "{base}");
        }
        assert!(spy.0.lock().unwrap().is_empty(), "no call over plain HTTP");
        assert!(matches!(
            fetch(&spy, API_BASE, &files(), &key, &logo, &|| false),
            Err(BrandfetchError::NoLogo)
        ));
        check(&spy, API_BASE, &key).unwrap();
        let calls = spy.0.lock().unwrap().clone();
        assert_eq!(
            calls,
            [
                (
                    "https://api.brandfetch.io/v2/brands/example.com".into(),
                    true
                ),
                (
                    "https://api.brandfetch.io/v2/brands/brandfetch.com".into(),
                    true
                ),
            ]
        );
        assert!(calls.iter().all(|(url, _)| !url.contains("test-api-key")));
    }

    #[test]
    fn brands_are_domains_or_ids_only() {
        for good in ["example.com", "id_0dwKPKT", "sub.example-site.org"] {
            assert!(valid_brand(good), "{good}");
        }
        for bad in ["", "../x", "a/b", "a?b", ".x", "-x", "a b", "a%2fb"] {
            assert!(!valid_brand(bad), "{bad}");
        }
    }
}
