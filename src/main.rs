// TODO: Run clippy, and add a #warn for it

use structopt::StructOpt;

use std::fs::File;
use std::io::{BufWriter, Write};
#[cfg(feature = "server")]
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time;

mod commits;
mod find_repositories;
mod log;
mod render;
#[cfg(feature = "server")]
mod server;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ProjectMetadata {
    name: String,
    path: PathBuf,
}

#[derive(Clone, Default)]
pub struct Day {
    filler: bool,
    commits: Vec<ProjectMetadata>,
}

#[derive(Clone)]
pub struct Year {
    year: i32,
    days: Vec<Day>,
}

#[derive(StructOpt)]
#[structopt(author)]
#[structopt(about)]
/// Generates a nice activity graph from a bunch of Git repositories
pub struct Args {
    #[structopt(subcommand)]
    command: Option<CommandArgs>,
    /// Prints verbose information
    #[structopt(short, long)]
    verbose: bool,
    /// Prints a visualization into stdout
    #[structopt(long)]
    stdout: bool,
    #[structopt(flatten)]
    gen: GenerationData,
    #[structopt(flatten)]
    ext: ExternalResources,
}

#[derive(StructOpt, Default, Clone)]
pub struct GenerationData {
    /// Regex that matches the author(s) whose commits are being
    /// counted (if not set, all commits will be counted)
    #[structopt(short, long)]
    author: Option<String>,
    /// How many subdirectories deep the program should search (if not
    /// set, there is no limit)
    #[structopt(short, long)]
    depth: Option<i32>,
    /// Path(s) to the directory (or directories) containing the
    /// repositories you want to include
    #[structopt(short, long)]
    input: Vec<PathBuf>,
}

#[derive(StructOpt, Clone, Default)]
pub struct ExternalResources {
    /// A html file that will be pasted in the <head> element
    #[structopt(long)]
    external_head: Option<PathBuf>,
    /// A html file that will be pasted at the beginning of the <body>
    /// element
    #[structopt(long)]
    external_header: Option<PathBuf>,
    /// A html file that will be pasted at the end of the <body>
    /// element
    #[structopt(long)]
    external_footer: Option<PathBuf>,
    /// A css file that will be pasted at the end of the css
    #[structopt(long)]
    external_css: Option<PathBuf>,
}

#[derive(StructOpt)]
enum CommandArgs {
    /// Output the generated html into a file
    Generate {
        /// The file that the resulting html will be printed out to
        #[structopt(short = "o", long, default_value = "activity-graph.html")]
        html: PathBuf,
        /// The file that the stylesheet will be printed out to (if not
        /// set, it will be included in the html inside a style-element)
        #[structopt(short, long)]
        css: Option<PathBuf>,
    },

    #[cfg(feature = "server")]
    /// Run a server that serves the generated activity graph html
    Server {
        /// The address that the server is hosted on
        #[structopt(long, default_value = "127.0.0.1:80")]
        host: SocketAddr,
        /// The minimum amount of seconds between regenerating the
        /// html and css
        #[structopt(long, default_value = "1")]
        cache_lifetime: u64,
    },
}

fn main() {
    let start_time = time::Instant::now();
    let args = Args::from_args();
    log::set_verbosity(args.verbose);

    let stdout_years;

    if let Some(command) = &args.command {
        match command {
            CommandArgs::Generate { html, css } => {
                let write_to_file = |path: &Path, s: String, name: &str| {
                    let mut writer = File::create(path).map(|file| BufWriter::new(file));
                    match &mut writer {
                        Ok(writer) => {
                            if let Err(err) = writer.write(&s.as_bytes()) {
                                eprintln!(
                                    "error: encountered while writing out the {}: {}",
                                    name, err
                                );
                            }
                        }
                        Err(err) => {
                            eprintln!(
                                "error: encountered while creating the {} file: {}",
                                name, err
                            );
                        }
                    }
                };

                let years = generate_years(&args.gen);

                let output_html = render::html(&args.ext, &html, css.as_ref(), &years);
                write_to_file(&html, output_html, "html");

                if let Some(css) = css {
                    let output_css = render::css(&args.ext);
                    write_to_file(&css, output_css, "css");
                }

                stdout_years = years;
            }

            #[cfg(feature = "server")]
            CommandArgs::Server {
                host,
                cache_lifetime,
            } => {
                server::run(&args, *host, *cache_lifetime);
                return;
            }
        }
    } else {
        stdout_years = generate_years(&args.gen);
    }

    if args.stdout {
        println!("{}", render::ascii(&stdout_years));
    }

    log::verbose_println(
        &format!(
            "finished all tasks, this run of the program took {:?}",
            time::Instant::now() - start_time
        ),
        false,
    );
}

pub fn generate_years(gen: &GenerationData) -> Vec<Year> {
    let repos = find_repositories::from_paths(&gen.input, gen.depth);
    let commit_dates = commits::find_dates(gen.author.as_ref(), &repos);
    render::gather_years(commit_dates)
}
