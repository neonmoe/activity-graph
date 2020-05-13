use hyper::header::{HeaderValue, CONTENT_TYPE};
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, Server, StatusCode};
use tokio::runtime::Runtime;
use tokio::task;

use std::convert::Infallible;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use std::time::{Duration, Instant};

use crate::{generate_years, log, render, ExternalResources, GenerationData};

lazy_static::lazy_static! {
    // These are set before the server is run, and only used in responses
    static ref GENERATION_DATA: RwLock<GenerationData> = RwLock::new(GenerationData::default());
    static ref EXTERNAL_HTML: RwLock<ExternalResources> = RwLock::new(ExternalResources::default());
    static ref LAST_CACHE: RwLock<Instant> = RwLock::new(Instant::now());
    static ref CACHE_LIFETIME: RwLock<Duration> = RwLock::new(Duration::from_secs(0));

    // A backup of the current CACHED_HTML and CACHED_CSS values on
    // disk. Encoded in the order: <html> <CACHE_FILE_SPLITTER> <css>
    static ref CACHE_FILE: RwLock<Option<PathBuf>> = RwLock::new(None);
    static ref CACHED_HTML: RwLock<String> = RwLock::new(String::new());
    static ref CACHED_CSS: RwLock<String> = RwLock::new(String::new());
}

static REFRESHING_CACHE: AtomicBool = AtomicBool::new(false);
static CACHE_INITIALIZED: AtomicBool = AtomicBool::new(false);

static INDEX_PATHS: &[&str] = &["/", "/index.html", "/index.htm", ""];

// This is invalid UTF-8, and so can be used as a delimiter between
// Strings, as Strings are always valid UTF-8.
const CACHE_FILE_SPLITTER: u8 = 0xFE;

pub fn run(
    gen: &GenerationData,
    ext: &ExternalResources,
    cache_file: Option<PathBuf>,
    host: SocketAddr,
    cache_lifetime: u64,
) {
    log::verbose_println(&format!("starting server on {}...", host), true);

    if let (Ok(mut gen_), Ok(mut ext_), Ok(mut cache_file_), Ok(mut lifetime), Ok(mut last_cache)) = (
        GENERATION_DATA.write(),
        EXTERNAL_HTML.write(),
        CACHE_FILE.write(),
        CACHE_LIFETIME.write(),
        LAST_CACHE.write(),
    ) {
        *gen_ = gen.clone();
        *ext_ = ext.clone();
        *cache_file_ = cache_file;
        *lifetime = Duration::from_secs(cache_lifetime);
        *last_cache = Instant::now() - Duration::from_secs(cache_lifetime * 2);
    } else {
        unreachable!();
    }

    match Runtime::new() {
        Ok(mut runtime) => {
            runtime.block_on(async {
                let make_service =
                    make_service_fn(|_conn| async { Ok::<_, Infallible>(service_fn(handle)) });
                let server = Server::bind(&host).serve(make_service);
                log::println(&format!("server started on {}", host));
                if let Err(err) = server.await {
                    log::println(&format!(
                        "error: hyper server encountered an error: {}",
                        err
                    ));
                }
            });
        }
        Err(err) => {
            log::println(&format!("error: could not start tokio runtime: {}", err));
        }
    }
}

async fn handle(req: Request<Body>) -> Result<Response<Body>, Infallible> {
    let (cache, mime_type) = if INDEX_PATHS.contains(&req.uri().path()) {
        refresh_caches().await;
        (CACHED_HTML.read(), HeaderValue::from_static("text/html"))
    } else if req.uri() == "/activity-graph.css" {
        refresh_caches().await;
        (CACHED_CSS.read(), HeaderValue::from_static("text/css"))
    } else {
        return Ok(error_response("404 Not Found", StatusCode::NOT_FOUND));
    };
    if let Ok(cache) = cache {
        let mut response = Response::new(Body::from(cache.clone()));
        response.headers_mut().insert(CONTENT_TYPE, mime_type);
        Ok(response)
    } else {
        Ok(error_response(
            "500 Internal Server Error\nSorry, the server encountered an unexpected error.",
            StatusCode::INTERNAL_SERVER_ERROR,
        ))
    }
}

fn error_response(s: &'static str, status_code: StatusCode) -> Response<Body> {
    let mut response = Response::new(Body::from(s));
    *response.status_mut() = status_code;
    response
}

async fn refresh_caches() {
    task::spawn_blocking(|| {
        let refresh_time = {
            let last_cache = LAST_CACHE.read().unwrap();
            let lifetime = CACHE_LIFETIME.read().unwrap();
            *last_cache + *lifetime
        };
        if Instant::now() >= refresh_time
            && !REFRESHING_CACHE.compare_and_swap(false, true, Ordering::Relaxed)
        {
            log::verbose_println("refreshing cache...", false);

            // Load from cache file if the cache has not been
            // initialized yet (if it exists)
            if !CACHE_INITIALIZED.load(Ordering::Relaxed) {
                if let Some((html, css)) = read_cache_file() {
                    if let (Ok(mut html_cache), Ok(mut css_cache)) =
                        (CACHED_HTML.write(), CACHED_CSS.write())
                    {
                        *html_cache = html;
                        *css_cache = css;
                        CACHE_INITIALIZED.store(true, Ordering::Relaxed);
                        log::println("initialized cache from cache file");
                    }
                }
            }

            let start = Instant::now();
            if let (Ok(gen), Ok(ext)) = (GENERATION_DATA.read(), EXTERNAL_HTML.read()) {
                let years = generate_years(&gen);
                let html_path = PathBuf::from("/index");
                let css_path = PathBuf::from("/activity-graph.css");
                let output_html = render::html(&ext, &html_path, Some(&css_path), &years);
                let output_css = render::css(&ext);

                let (cache_html, cache_css) = (output_html.clone(), output_css.clone());
                task::spawn(async move {
                    if let Err(err) = write_cache_file(&cache_html, &cache_css) {
                        log::println(&format!(
                            "error: ran into an IO error while writing cache file: {}",
                            err
                        ));
                    }
                });

                if let Ok(mut html) = CACHED_HTML.write() {
                    *html = output_html;
                }
                if let Ok(mut css) = CACHED_CSS.write() {
                    *css = output_css;
                }
                if let Ok(mut last_cache) = LAST_CACHE.write() {
                    *last_cache = Instant::now();
                }
            }
            log::println(&format!("updated cache, took {:?}", Instant::now() - start));

            REFRESHING_CACHE.store(false, Ordering::Relaxed); // Allow future refreshes
            CACHE_INITIALIZED.store(true, Ordering::Relaxed); // Allow early requests to complete
        }
    });

    // Yield until the cache has been initialized
    while !CACHE_INITIALIZED.load(Ordering::Relaxed) {
        task::yield_now().await;
    }
}

fn write_cache_file(html: &str, css: &str) -> Result<(), io::Error> {
    if let Ok(cache_file) = CACHE_FILE.read() {
        if cache_file.is_some() {
            log::verbose_println("writing cache file...", true);
            let file = File::create(cache_file.as_ref().unwrap())?;
            let mut writer = BufWriter::new(file);
            write!(writer, "ACTIVITY-GRAPH-CACHE-FILE")?;
            writer.write(&[CACHE_FILE_SPLITTER])?;
            write!(writer, "{}", html)?;
            writer.write(&[CACHE_FILE_SPLITTER])?;
            write!(writer, "{}", css)?;
            drop(writer); // This should flush out the file write
            log::verbose_println("wrote cache file", false);
        }
    }
    Ok(())
}

fn read_cache_file() -> Option<(String, String)> {
    if let Ok(cache_file) = CACHE_FILE.read() {
        if cache_file.is_some() {
            let file = File::open(cache_file.as_ref().unwrap());
            match file {
                Ok(file) => {
                    let mut reader = BufReader::new(file);
                    let mut bytes = Vec::new();
                    if reader.read_to_end(&mut bytes).is_ok() {
                        // Split at CACHE_FILE_SPLITTER and return the
                        // parts between as `&str`s.
                        let parts: Vec<&str> = bytes
                            .split(|b| *b == CACHE_FILE_SPLITTER)
                            .filter_map(|bytes: &[u8]| std::str::from_utf8(bytes).ok())
                            .collect();
                        if parts.len() == 3 {
                            let (magic, html, css) = (parts[0], parts[1], parts[2]);
                            if magic == "ACTIVITY-GRAPH-CACHE-FILE" {
                                return Some((html.to_string(), css.to_string()));
                            }
                        }
                    }
                }
                Err(err) => {
                    log::println(&format!("error: could not read cache file: {}", err));
                }
            }
        }
    }
    None
}
