use chrono::{DateTime, Datelike, Utc};
use structopt::StructOpt;

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time;

mod commits;
mod find_repositories;
mod log;
mod render;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ProjectMetadata {
    name: String,
    path: PathBuf,
}

#[derive(Clone, Default)]
pub struct Day<'a> {
    commits: Vec<&'a ProjectMetadata>,
}

#[derive(Clone)]
pub struct Year<'a> {
    year: i32,
    days: Vec<Day<'a>>,
}

#[derive(StructOpt)]
#[structopt(author)]
#[structopt(about)]
/// Generates a nice activity graph from a bunch of Git repositories
pub struct Args {
    /// Prints verbose information
    #[structopt(short, long)]
    verbose: bool,
    /// Prints a visualization into stdout. This is implied if an
    /// output file is not specified
    #[structopt(short, long)]
    stdout: bool,

    /// Regex that matches the author(s) whose commits are being
    /// counted (if not set, all commits will be accounted for)
    #[structopt(short, long)]
    author: Option<String>,
    /// How many subdirectories deep the program should search (if not
    /// set, there is no limit)
    #[structopt(short, long)]
    depth: Option<i32>,
    /// The file that the resulting html will be printed out to
    #[structopt(short, long)]
    output: Option<PathBuf>,
    /// The file that the stylesheet will be printed out to (if not
    /// set, it will be included in the html inside a style-element)
    #[structopt(short, long)]
    css: Option<PathBuf>,
    /// Path(s) to the directory (or directories) containing the
    /// repositories you want to include
    #[structopt(short, long)]
    repos: Vec<PathBuf>,

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
}

fn main() {
    let start_time = time::Instant::now();
    let mut args = Args::from_args();

    if !args.verbose && args.output.is_none() {
        args.stdout = true;
    }

    log::set_verbosity(args.verbose);

    let repos = find_repositories::from_paths(&args.repos, args.depth);

    let mut commit_dates = commits::find_dates(args.author.as_ref(), &repos);
    commit_dates.sort_by(|(a, _), (b, _)| a.cmp(b));

    let years = if commit_dates.len() > 0 {
        let get_year = |date: DateTime<Utc>| date.date().iso_week().year();
        let first_year = get_year(commit_dates[0].0);
        let last_year = get_year(commit_dates[commit_dates.len() - 1].0);
        render::gather_years(&commit_dates, first_year, last_year)
    } else {
        Vec::new()
    };

    let output_html = render::html(&args, &years);
    let mut writer = args
        .output
        .as_ref()
        .and_then(|path| File::create(path).ok())
        .map(|file| BufWriter::new(file));
    if let Some(writer) = &mut writer {
        if let Err(err) = writer.write(&output_html.as_bytes()) {
            eprintln!("error: encountered while writing out the html: {}", err);
        }
    }

    let output_css = render::css();
    let mut writer = args
        .css
        .as_ref()
        .and_then(|path| File::create(path).ok())
        .map(|file| BufWriter::new(file));
    if let Some(writer) = &mut writer {
        if let Err(err) = writer.write(&output_css.as_bytes()) {
            eprintln!("error: encountered while writing out the css: {}", err);
        }
    }

    if args.stdout {
        println!("{}", render::ascii(&years));
    }

    log::verbose_println(
        &format!(
            "finished all tasks, this run of the program took {:?}",
            time::Instant::now() - start_time
        ),
        false,
    );
}
