// TODO: Link the individual squares to some anchor which should then display a small summary of what commits that day contained
// TOOD: A --host flag that allows, hosting a http server that returns the generated html

use chrono::{DateTime, Datelike, Utc};
use rayon::prelude::*;
use structopt::StructOpt;

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time;

static HTML_HEAD: &str =
    "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><title>Activity</title></head><body>";
static HTML_TAIL: &str = "</body>";
static CSS: &str = include_str!("main.css");

#[derive(Clone)]
struct Day<'a> {
    commits: Vec<&'a ProjectMetadata>,
}

#[derive(Clone)]
struct Year<'a> {
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
    /// How many subdirectories deep the program should search (if not specified, there is no limit)
    #[structopt(short, long)]
    depth: Option<i32>,
    /// Regex that matches the author(s) whose commits are being
    /// counted
    #[structopt(short, long)]
    author: String,
    /// The file that the resulting html will be printed out to
    #[structopt(short, long)]
    output: Option<PathBuf>,
    /// The file that the stylesheet will be printed out to (if not
    /// provided, it will be included in the html inside a
    /// style-element)
    #[structopt(short, long)]
    css: Option<PathBuf>,
    /// Path(s) to the directory (or directories) containing the
    /// repositories you want to include
    #[structopt(short, long)]
    repos: Vec<PathBuf>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ProjectMetadata {
    name: String,
    path: PathBuf,
}

fn main() {
    let start_time = time::Instant::now();
    let mut args = Args::from_args();

    if !args.verbose && args.output.is_none() {
        args.stdout = true;
    }

    // Don't mind this verbose printing stuff, it's just a very
    // elaborate system to print "updating" lines with a cooldown.
    let last_update_print = Mutex::new(time::Instant::now());
    let last_verbose_print_was_update = Mutex::new(false);
    let verbose_println = |s: &str, updating_line: bool| {
        if args.verbose {
            let width = term_size::dimensions()
                .map(|(w, _)| w - 1)
                .unwrap_or(70)
                .max(4);

            if updating_line {
                // Throttle the line updates to once per 20ms, 50 Hz is plenty real-time.
                if let Ok(mut last_update) = last_update_print.lock() {
                    let now = time::Instant::now();
                    if now - *last_update < time::Duration::from_millis(20) {
                        return;
                    } else {
                        *last_update = now;
                    }
                }

                // Clear the line, then write the line, but limit it to the terminal width
                if s.len() >= width - 1 {
                    eprint!("{:width$}\r{}...\r", "", &s[..width - 4], width = width);
                } else {
                    eprint!("{:width$}\r{}\r", "", s, width = width);
                };
                if let Ok(mut was_update) = last_verbose_print_was_update.lock() {
                    *was_update = true;
                }
            } else {
                if let Ok(mut was_update) = last_verbose_print_was_update.lock() {
                    if *was_update {
                        // Clear the line
                        eprint!("{:width$}\r", "", width = width);
                    }
                    *was_update = false;
                }
                eprintln!("{}", s);
            }
        }
    };

    for repo_dir in &args.repos {
        match fs::read_dir(&repo_dir) {
            Ok(subdirs) => {
                // Find all the repository directories
                let mut repos = HashSet::new();
                analyze_dirs(&mut repos, &repo_dir, subdirs, args.depth, &verbose_println);
                verbose_println("finished scanning for git repositories", false);

                let commit_count = AtomicU32::new(0);
                let author = format!("--author={}", args.author);
                let mut commit_dates = repos
                    .par_iter()
                    // Collect the metadata of each commit in the repositories
                    .map(|repo| {
                        let mut commit_dates: Vec<(DateTime<Utc>, &ProjectMetadata)> = Vec::new();
                        let path = &repo.path;
                        let commits =
                            run_git(&path, &["log", "--all", "--format=oneline", &author]);
                        let lines: Vec<&str> = commits
                            .lines()
                            .filter_map(|line| line.split(' ').next())
                            .collect();
                        commit_dates.par_extend(lines.par_iter().filter_map(|hash| {
                            let date = run_git(
                                &path,
                                &["show", "-s", "--format=%ai", "--date=iso8601", &hash],
                            );
                            if let Ok(date) = date.parse() {
                                // Note: Chrono adheres to ISO 8601, and
                                // that's what we ask from git, so this
                                // should always be valid.
                                let count = commit_count.fetch_add(1, Ordering::Relaxed) + 1;
                                verbose_println(
                                    &format!("commits accounted for {}\r", count),
                                    true,
                                );
                                Some((date, repo))
                            } else {
                                None
                            }
                        }));
                        commit_dates
                    })
                    // Fold all the gathered dates into one vec
                    .reduce(
                        || Vec::new(),
                        |mut a, b| {
                            a.extend(&b);
                            a
                        },
                    );

                verbose_println(
                    &format!(
                        "counted up {} commits in {} repositories",
                        commit_dates.len(),
                        repos.len()
                    ),
                    false,
                );

                // Sort the metadata by date, and prepare to go through all of them
                commit_dates.sort_by(|(a, _), (b, _)| a.cmp(b));
                let get_year = |date: DateTime<Utc>| date.date().iso_week().year();
                let (first_year, last_year) = if commit_dates.len() > 0 {
                    (
                        get_year(commit_dates[0].0),
                        get_year(commit_dates[commit_dates.len() - 1].0),
                    )
                } else {
                    // There are no commits, so it doesn't really matter what the years are.
                    (2000, 2000)
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
                            let day_index = weekday_index * weeks + week_index;
                            days[day_index].commits.push(*metadata);
                        }

                        i += 1;
                    }

                    verbose_println(
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

                if let Some(output_path) = &args.output {
                    let mut output = File::create(output_path).map(|file| BufWriter::new(file));
                    if let Ok(writer) = &mut output {
                        let relative_path = args
                            .css
                            .as_ref()
                            .and_then(|css_path| {
                                if let Some(base) = output_path.parent() {
                                    pathdiff::diff_paths(&css_path, base)
                                } else {
                                    Some(css_path.to_path_buf())
                                }
                            })
                            .map(create_web_path);
                        render_to_file(&years, weeks, relative_path, writer, &verbose_println);
                    }
                }

                let mut css = args
                    .css
                    .as_ref()
                    .and_then(|path| File::create(path).ok())
                    .map(|file| BufWriter::new(file));
                if let Some(writer) = &mut css {
                    let _ = write!(writer, "{}", CSS);
                }

                if args.stdout {
                    render_to_stdout(&years, weeks, &verbose_println);
                }

                verbose_println(
                    &format!(
                        "finished all tasks, this run of the program took {:?}",
                        time::Instant::now() - start_time
                    ),
                    false,
                );
            }
            Err(err) => eprintln!("error: cannot read directory ({})", err),
        }
    }
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

fn render_to_file<F: Fn(&str, bool)>(
    years: &[Year],
    weeks: usize,
    css_path: Option<String>,
    writer: &mut BufWriter<File>,
    verbose_println: &F,
) {
    verbose_println("writing html to file...", true);

    let head = if let Some(css_path) = css_path {
        HTML_HEAD.replace(
            "</head>",
            &format!("<link href=\"{}\" rel=\"stylesheet\"></head>", css_path),
        )
    } else {
        HTML_HEAD.replace("</head>", &format!("<style>{}</style></head>", CSS))
    };
    let _ = writeln!(writer, "{}", head);

    for year in years.iter().rev() {
        let max_count = get_max_count(year);
        let _ = writeln!(
            writer,
            "<table class=\"activity-table\"><thead><tr><td class=\"activity-header-year\" colspan=\"{}\">{}</td></tr></thead><tbody>",
            weeks, year.year
        );
        for day in 0..7 {
            let _ = writeln!(writer, "<tr>");
            for week in 0..weeks {
                let metadata = &year.days[day * weeks + week];
                let shade = get_shade_class(metadata.commits.len(), max_count);
                let _ = writeln!(
                    writer,
                    "<td class=\"activity-blob activity-level-{}\"></td>",
                    shade
                );
            }
            let _ = writeln!(writer, "</tr>");
        }
        let _ = writeln!(writer, "</table></tbody>");
    }
    let _ = writeln!(writer, "{}", HTML_TAIL);
    verbose_println("wrote html to file", false);
}

fn render_to_stdout<F: Fn(&str, bool)>(years: &[Year], weeks: usize, verbose_println: &F) {
    verbose_println("writing ascii representation to stdout...", true);
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
    verbose_println("wrote ascii representation to stdout", false);
}

fn run_git(work_dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(work_dir)
        .stdout(Stdio::piped())
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn path_to_string(path: &Path) -> Option<String> {
    path.canonicalize()
        .ok()
        .map(|path| path.into_os_string())
        .and_then(|path| path.into_string().ok())
}

fn analyze_dirs<F: Fn(&str, bool)>(
    git_paths: &mut HashSet<ProjectMetadata>,
    path: &Path,
    dirs: fs::ReadDir,
    depth: Option<i32>,
    verbose_println: &F,
) {
    if let Some(path) = path_to_string(&path) {
        verbose_println(&format!("scanning: {}\r", path), true);
    }

    let dirs: Vec<fs::DirEntry> = dirs.filter_map(|result| result.ok()).collect();
    if dirs
        .iter()
        .map(|dir_entry| dir_entry.file_name())
        .any(|file_name| file_name == ".git")
    {
        if let Some(name) = path
            .file_name()
            .and_then(|os_str| os_str.to_str())
            .map(|s| s.to_string())
        {
            git_paths.insert(ProjectMetadata {
                name,
                path: PathBuf::from(&path),
            });
        }
    }

    if depth.iter().any(|depth| *depth < 0) {
        // Too deep, don't search subdirectories.
        return;
    }

    for dir in dirs {
        let path = dir.path();
        if path.file_name().iter().any(|name| *name != ".git") {
            let fix_symlink = |link_path: PathBuf| {
                // Fill out the path if it's relative, because it's
                // relative to the path variable (at least on windows,
                // should probably test this on other OSes as well,
                // but, well, I rarely use symlinks).
                if let Some(base) = path.parent() {
                    if !link_path.is_absolute() {
                        let mut fixed_symlink = PathBuf::from(base);
                        fixed_symlink.push(link_path);
                        return fixed_symlink;
                    }
                }
                link_path
            };
            let path = fs::read_link(&path).map(fix_symlink).unwrap_or(path);
            if let Ok(dirs) = fs::read_dir(&path) {
                analyze_dirs(
                    git_paths,
                    &path,
                    dirs,
                    depth.map(|depth| depth - 1),
                    verbose_println,
                );
            }
        }
    }
}
