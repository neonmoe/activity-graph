//! Contains the functionality to render the visualizations out of
//! dated commit data.
use chrono::{DateTime, Datelike, Utc};

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Component, PathBuf};

use crate::{log, Day, ExternalResources, ProjectMetadata, Year};

static HTML_HEAD: &str = include_str!("head.html");
static CSS: &str = include_str!("activity-graph.css");
static WEEKS: usize = 53;

pub fn gather_years(
    commit_dates: Vec<(DateTime<Utc>, ProjectMetadata)>,
    first_year: i32,
    last_year: i32,
) -> Vec<Year> {
    // Years is a vec containing vecs of years, which consist
    // of weekday-major grids of days: eg. the first row
    // represents all of the mondays in the year, in order.
    let mut years = Vec::with_capacity((last_year - first_year + 1) as usize);
    for year in first_year..=last_year {
        years.push(Year {
            year,
            days: vec![Day::default(); WEEKS * 7],
        });
    }

    let mut commit_dates = commit_dates.into_iter();
    let mut counted_commits = 0;
    for year in first_year..=last_year {
        // Loop through the years

        let days = &mut years[(year - first_year) as usize].days;
        while let Some((date, metadata)) = commit_dates.next() {
            // Loop through the days until the commit is from
            // next year or commits run out

            if date.iso_week().year() != year {
                break;
            }
            let weekday_index = date.weekday().num_days_from_monday() as usize;
            let week_index = date.iso_week().week0() as usize;
            if week_index < WEEKS {
                let day = &mut days[weekday_index * WEEKS + week_index];
                day.commits.push(metadata);
                counted_commits += 1;
            }
        }

        log::verbose_println(
            &format!(
                "prepared year {} for rendering, {} commits processed so far",
                year, counted_commits
            ),
            false,
        );
    }
    years
}

/// Renders a HTML visualization of the commits based on the
/// arguments.
pub fn html(
    ext: &ExternalResources,
    html_path: &PathBuf,
    css_path: Option<&PathBuf>,
    years: &[Year],
) -> String {
    // Prepare the html scaffolding around the tables
    let external_head = read_optional_file(&ext.external_head).unwrap_or_else(String::new);
    let external_header = read_optional_file(&ext.external_header).unwrap_or_else(String::new);
    let external_footer = read_optional_file(&ext.external_footer).unwrap_or_else(String::new);
    let external_css = read_optional_file(&ext.external_css).unwrap_or_else(String::new);

    let mut style = None;
    if let (Some(base), Some(css_path)) = (html_path.parent(), &css_path) {
        if let Some(relative_path) = pathdiff::diff_paths(&css_path, base) {
            // Add the <link> element instead of <style> if using external css
            let path = create_web_path(relative_path);
            style = Some(format!("<link href=\"{}\" rel=\"stylesheet\">", path));
        }
    }
    if style.is_none() {
        style = Some(format!("<style>\n{}\n{}</style>", CSS, external_css));
    }
    let style = style.unwrap();

    let head = format!(
        "<!DOCTYPE html>\n<html>\n<head>\n{}\n{}\n{}\n</head>\n<body>\n{}\n",
        HTML_HEAD, style, external_head, external_header
    );
    let tail = format!("{}</body></html>", external_footer);

    // Render the tables
    let mut result = String::with_capacity(1024);
    log::verbose_println("rendering html...", true);
    result += &head;
    for year in years.iter().rev() {
        let max_count = get_max_count(year);
        result += &format!(
            "<table class=\"activity-table\"><thead><tr><td class=\"activity-header-year\" colspan=\"{}\"><h3>{}</h3></td></tr></thead><tbody>\n",
            WEEKS, year.year
        );
        for day in 0..7 {
            result += "<tr>";
            for week in 0..WEEKS {
                let metadata = &year.days[day * WEEKS + week];
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
    result += &tail;
    log::verbose_println("rendered html", false);
    result
}

pub fn css(ext: &ExternalResources) -> String {
    let external_css = read_optional_file(&ext.external_css).unwrap_or_else(String::new);
    format!("{}\n{}", CSS, external_css)
}

/// Renders an ASCII visualization of the commits.
pub fn ascii(years: &[Year]) -> String {
    let mut result = String::with_capacity(512);
    log::verbose_println("rendering ascii visualization...", true);
    for year in years.iter().rev() {
        let max_count = get_max_count(year);
        result.push('\n');
        for day in 0..7 {
            for week in 0..WEEKS {
                let metadata = &year.days[day * WEEKS + week];
                let shade = metadata.commits.len() as f32 / max_count as f32;
                result.push(get_shaded_char(shade));
            }
            result.push('\n');
        }
    }
    log::verbose_println("rendered ascii visualization", false);
    result
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
