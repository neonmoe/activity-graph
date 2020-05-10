use chrono::{DateTime, Datelike, Utc};
use rayon::prelude::*;
use structopt::StructOpt;

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time;

static HTML_HEAD: &str = include_str!("head.html");
static CSS: &str = include_str!("activity-graph.css");

#[derive(Clone, PartialEq, Eq, Hash)]
struct ProjectMetadata {
    name: String,
    path: PathBuf,
}

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

    // Find the repositories
    let repos = args
        .repos
        .iter()
        .map(|repo_dir| {
            match fs::read_dir(&repo_dir) {
                Ok(subdirs) => {
                    // Find all the repository directories
                    let mut repos = HashSet::new();
                    analyze_dirs(&mut repos, &repo_dir, subdirs, args.depth, &verbose_println);
                    repos
                }
                Err(err) => {
                    eprintln!("error: cannot read directory ({})", err);
                    HashSet::new()
                }
            }
        })
        .fold(HashSet::new(), |mut a, b| {
            a.extend(b);
            a
        });
    verbose_println("finished scanning for git repositories", false);

    // Count up the commits in all of the repositories
    let commit_count = AtomicU32::new(0);
    let author_flag = args
        .author
        .as_ref()
        .map(|author| format!("--author={}", author));
    let mut commit_dates = repos
        .par_iter()
        // Collect the metadata of each commit in the repositories
        .map(|repo| {
            let mut commit_dates: Vec<(DateTime<Utc>, &ProjectMetadata)> = Vec::new();
            let path = &repo.path;
            let mut args = vec!["log", "--all", "--format=oneline"];
            if let Some(author_flag) = &author_flag {
                args.push(author_flag);
            }
            let commits = run_git(&path, &args);
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
                    verbose_println(&format!("commits accounted for {}\r", count), true);
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

    let html = render_to_html(&years, weeks, &html_head, &html_tail, &verbose_println);

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

fn render_to_html<F: Fn(&str, bool)>(
    years: &[Year],
    weeks: usize,
    head: &str,
    tail: &str,
    verbose_println: &F,
) -> String {
    let mut result = String::with_capacity(1024);
    verbose_println("rendering html to string...", true);

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
    verbose_println("rendered html to string", false);
    result
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
