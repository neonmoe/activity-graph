use chrono::{DateTime, Datelike, Utc};
use structopt::StructOpt;

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Component, PathBuf};
use std::time;

mod commits;
mod find_repositories;
mod log;

static HTML_HEAD: &str = include_str!("head.html");
static CSS: &str = include_str!("activity-graph.css");

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ProjectMetadata {
    name: String,
    path: PathBuf,
}

#[derive(Clone)]
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
struct Args {
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

    // Sort the commits by date, and prepare to go through all of them
    commit_dates.sort_by(|(a, _), (b, _)| a.cmp(b));
    let get_year = |date: DateTime<Utc>| date.date().iso_week().year();
    let (no_commits, first_year, last_year) = if commit_dates.len() > 0 {
        (
            false,
            get_year(commit_dates[0].0),
            get_year(commit_dates[commit_dates.len() - 1].0),
        )
    } else {
        // There are no commits, so it doesn't really matter what the years are.
        (true, 2000, 2000)
    };

    // Since the graph is ISO week based, there might be 52 or
    // 53 weeks per year. Might clip off the last year during
    // leap years to achieve a more consistent look, we shall
    // see.
    let weeks = 53;
    // Years is a vec containing vecs of years, which consist
    // of weekday-major grids of days: eg. the first row
    // represents all of the mondays in the year, in order.
    let mut years = Vec::with_capacity((last_year - first_year + 1) as usize);
    for year in first_year..=last_year {
        if no_commits {
            break;
        }
        years.push(Year {
            year,
            days: vec![
                Day {
                    commits: Vec::new()
                };
                weeks * 7
            ],
        });
    }

    // Organize the metadata so it's easy to go through when rendering the graphs
    let mut i = 0;
    for year in first_year..=last_year {
        if no_commits {
            break;
        }

        // Loop through the years

        let days = &mut years[(year - first_year) as usize].days;
        while i < commit_dates.len() {
            // Loop through the days until the commit is from
            // next year or commits run out

            let (date, metadata) = &commit_dates[i];
            if date.iso_week().year() != year {
                break;
            }
            let weekday_index = date.weekday().num_days_from_monday() as usize;
            let week_index = date.iso_week().week0() as usize;
            if week_index < weeks {
                let day = &mut days[weekday_index * weeks + week_index];
                day.commits.push(*metadata);
            }

            i += 1;
        }

        log::verbose_println(
            &format!(
                "prepared year {} for rendering, {} commits processed so far",
                year, i
            ),
            false,
        );
        if i >= commit_dates.len() {
            break;
        }
    }

    // Load the head, header and footer files, and create the site's
    // html templates.

    let external_head = read_optional_file(&args.external_head).unwrap_or_else(String::new);
    let external_header = read_optional_file(&args.external_header).unwrap_or_else(String::new);
    let external_footer = read_optional_file(&args.external_footer).unwrap_or_else(String::new);

    let mut style = None;
    if let (Some(css_path), Some(output_path)) = (&args.css, &args.output) {
        if let Some(base) = output_path.parent() {
            if let Some(relative_path) = pathdiff::diff_paths(&css_path, base) {
                // Add the <link> element instead of <style> if using external css
                let path = create_web_path(relative_path);
                style = Some(format!("<link href=\"{}\" rel=\"stylesheet\">", path));
            }
        }
    }
    if style.is_none() {
        style = Some(format!("<style>\n{}</style>", CSS));
    }
    let style = style.unwrap();

    let html_head = format!(
        "<!DOCTYPE html>\n<html>\n<head>\n{}\n{}\n{}\n</head>\n<body>\n{}\n",
        HTML_HEAD, style, external_head, external_header
    );
    let html_tail = format!("{}</body></html>", external_footer);

    let html = render_to_html(&years, weeks, &html_head, &html_tail);

    let mut output = args
        .output
        .as_ref()
        .and_then(|path| File::create(path).ok())
        .map(|file| BufWriter::new(file));
    if let Some(writer) = &mut output {
        if let Err(err) = writer.write(html.as_bytes()) {
            eprintln!("error: encountered while writing out the html: {}", err);
        }
    }

    let mut css = args
        .css
        .as_ref()
        .and_then(|path| File::create(path).ok())
        .map(|file| BufWriter::new(file));
    if let Some(writer) = &mut css {
        if let Err(err) = writer.write(CSS.as_bytes()) {
            eprintln!("error: encountered while writing out the css: {}", err);
        }
    }

    if args.stdout {
        render_to_stdout(&years, weeks);
    }

    log::verbose_println(
        &format!(
            "finished all tasks, this run of the program took {:?}",
            time::Instant::now() - start_time
        ),
        false,
    );
}

fn create_web_path(path: PathBuf) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(s) => s.to_str(),
            Component::CurDir => Some("."),
            Component::ParentDir => Some(".."),
            _ => None,
        })
        .fold(String::new(), |mut a, b| {
            if a.len() > 0 {
                a += "/";
            }
            a += b;
            a
        })
}

fn read_optional_file(path: &Option<PathBuf>) -> Option<String> {
    let path = path.as_ref()?;
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut result = Vec::new();
    reader.read_to_end(&mut result).ok()?;
    String::from_utf8(result).ok()
}

fn get_max_count(year: &Year) -> usize {
    year.days
        .iter()
        .map(|metadata| metadata.commits.len())
        .max()
        .unwrap_or(0)
        .max(1)
}

fn get_shade_class(commits: usize, max_count: usize) -> usize {
    let norm = commits as f32 / max_count as f32;
    match norm {
        x if x == 0.0 => 0,
        x if x < 0.25 => 1,
        x if x < 0.5 => 2,
        x if x < 0.75 => 3,
        _ => 4,
    }
}

fn get_shaded_char(shade: f32) -> char {
    match shade {
        x if x > 0.5 => '\u{2593}',
        x if x > 0.0 => '\u{2592}',
        _ => '\u{2591}',
    }
}

fn render_to_html(years: &[Year], weeks: usize, head: &str, tail: &str) -> String {
    let mut result = String::with_capacity(1024);
    log::verbose_println("rendering html to string...", true);

    result += head;
    for year in years.iter().rev() {
        let max_count = get_max_count(year);
        result += &format!(
            "<table class=\"activity-table\"><thead><tr><td class=\"activity-header-year\" colspan=\"{}\"><h3>{}</h3></td></tr></thead><tbody>\n",
            weeks, year.year
        );
        for day in 0..7 {
            result += "<tr>";
            for week in 0..weeks {
                let metadata = &year.days[day * weeks + week];
                let commit_count = metadata.commits.len();
                let shade = get_shade_class(commit_count, max_count);
                let tooltip = if commit_count == 0 {
                    String::from("No commits")
                } else {
                    format!("{} commits", commit_count)
                };
                result += &format!(
                    "<td class=\"blob lvl{}\" title=\"{}\"></td>",
                    shade, tooltip
                );
            }
            result += "</tr>\n";
        }
        result += "</tbody></table>\n";
    }
    result += tail;
    log::verbose_println("rendered html to string", false);
    result
}

fn render_to_stdout(years: &[Year], weeks: usize) {
    log::verbose_println("writing ascii representation to stdout...", true);
    for year in years.iter().rev() {
        let max_count = get_max_count(year);
        println!("");
        for day in 0..7 {
            for week in 0..weeks {
                let metadata = &year.days[day * weeks + week];
                let shade = metadata.commits.len() as f32 / max_count as f32;
                print!("{}", get_shaded_char(shade));
            }
            println!("");
        }
    }
    log::verbose_println("wrote ascii representation to stdout", false);
}
