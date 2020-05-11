use hyper::service::{make_service_fn, service_fn};
use hyper::header::{HeaderValue, CONTENT_TYPE};
use hyper::{Body, Request, Response, Server, StatusCode};
use tokio::runtime::Runtime;
use tokio::task;

use std::convert::Infallible;
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

    static ref CACHED_HTML: RwLock<String> = RwLock::new(String::new());
    static ref CACHED_CSS: RwLock<String> = RwLock::new(String::new());
}

static REFRESHING_CACHE: AtomicBool = AtomicBool::new(false);
static CACHE_INITIALIZED: AtomicBool = AtomicBool::new(false);

static INDEX_PATHS: &[&str] = &["/", "/index.html", "/index.htm", ""];

pub fn run(gen: &GenerationData, ext: &ExternalResources, host: SocketAddr, cache_lifetime: u64) {
    log::verbose_println(&format!("starting server on {}...", host), true);

    if let (Ok(mut gen_), Ok(mut ext_), Ok(mut lifetime), Ok(mut last_cache)) = (
        GENERATION_DATA.write(),
        EXTERNAL_HTML.write(),
        CACHE_LIFETIME.write(),
        LAST_CACHE.write(),
    ) {
        *gen_ = gen.clone();
        *ext_ = ext.clone();
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
                log::verbose_println(&format!("server started on {}", host), false);
                if let Err(err) = server.await {
                    eprintln!("error: hyper server encountered an error: {}", err);
                }
            });
        }
        Err(err) => {
            eprintln!("error: could not start tokio runtime: {}", err);
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
    let task = task::spawn_blocking(|| {
        let refresh_time = {
            let last_cache = LAST_CACHE.read().unwrap();
            let lifetime = CACHE_LIFETIME.read().unwrap();
            *last_cache + *lifetime
        };
        if Instant::now() >= refresh_time
            && !REFRESHING_CACHE.compare_and_swap(false, true, Ordering::Relaxed)
        {
            log::verbose_println("refreshing cache...", false);
            let start = Instant::now();
            if let (Ok(gen), Ok(ext)) = (GENERATION_DATA.read(), EXTERNAL_HTML.read()) {
                let years = generate_years(&gen);
                let html_path = PathBuf::from("/index");
                let css_path = PathBuf::from("/activity-graph.css");
                let output_html = render::html(&ext, &html_path, Some(&css_path), &years);
                let output_css = render::css(&ext);
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
            log::verbose_println(
                &format!("updated cache, took {:?}", Instant::now() - start),
                false,
            );
            REFRESHING_CACHE.store(false, Ordering::Relaxed);
        }
    });

    // If the cache hasn't been initialized yet, wait for the refresh
    // to run by `await`ing it.
    if !CACHE_INITIALIZED.load(Ordering::Relaxed) && task.await.is_ok() {
        CACHE_INITIALIZED.store(true, Ordering::Relaxed);
    }
}
