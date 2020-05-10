use tokio::runtime::Runtime;
use tokio::task;
use hyper::{Body, Request, Response, Server, StatusCode};
use hyper::service::{make_service_fn, service_fn};

use std::net::SocketAddr;
use std::convert::Infallible;
use std::sync::RwLock;
use std::time::{Instant, Duration};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{generate_years, log, render, Args, GenerationData, ExternalHtml};

lazy_static::lazy_static! {
    // These are set before the server is run, and only used in responses
    static ref GENERATION_DATA: RwLock<GenerationData> = RwLock::new(Default::default());
    static ref EXTERNAL_HTML: RwLock<ExternalHtml> = RwLock::new(Default::default());
    static ref LAST_CACHE: RwLock<Instant> = RwLock::new(Instant::now());
    static ref CACHE_LIFETIME: RwLock<Duration> = RwLock::new(Duration::from_secs(0));

    static ref CACHED_HTML: RwLock<String> = RwLock::new(String::new());
    static ref CACHED_CSS: RwLock<String> = RwLock::new(String::new());
}

static REFRESHING_CACHE: AtomicBool = AtomicBool::new(false);

static INDEX_PATHS: &[&str] = &["/", "/index.html", "/index.htm", ""];

pub fn run(args: &Args, host: SocketAddr, cache_lifetime: u64) {
    log::verbose_println(&format!("starting server on {}...", host), true);

    if let (Ok(mut gen), Ok(mut ext), Ok(mut lifetime), Ok(mut last_cache)) = (GENERATION_DATA.write(), EXTERNAL_HTML.write(), CACHE_LIFETIME.write(), LAST_CACHE.write()) {
        *gen = args.gen.clone();
        *ext = args.ext.clone();
        *lifetime = Duration::from_secs(cache_lifetime);
        *last_cache = Instant::now() - Duration::from_secs(cache_lifetime * 2);
    } else {
        unreachable!();
    }

    match Runtime::new() {
        Ok(mut runtime) => {
            runtime.block_on(async {
                let make_service = make_service_fn(|_conn| async {
                    Ok::<_, Infallible>(service_fn(handle))
                });
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
    let cache = if INDEX_PATHS.contains(&req.uri().path()) {
        refresh_caches().await;
        CACHED_HTML.read()
    } else if req.uri() == "/activity-graph.css" {
        refresh_caches().await;
        CACHED_CSS.read()
    } else {
        let mut response = Response::new(Body::from("404 Not Found"));
        *response.status_mut() = StatusCode::NOT_FOUND;
        return Ok(response);
    };
    if let Ok(cache) = cache {
        Ok(Response::new(Body::from(cache.clone())))
    } else {
        let mut response = Response::new(Body::from("internal server error"));
        *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        Ok(response)
    }
}

async fn refresh_caches() {
    task::block_in_place(|| {
        let refresh_time = {
            let last_cache = LAST_CACHE.read().unwrap();
            let lifetime = CACHE_LIFETIME.read().unwrap();
            *last_cache + *lifetime
        };
        if Instant::now() >= refresh_time && !REFRESHING_CACHE.compare_and_swap(false, true, Ordering::Relaxed) {
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
            log::verbose_println(&format!("updated cache, took {:?}", Instant::now() - start), false);
            REFRESHING_CACHE.store(false, Ordering::Relaxed);
        }
    })
}
