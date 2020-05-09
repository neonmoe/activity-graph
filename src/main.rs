// TODO: Add line-clearing and length-limits to the "updating" lines depending on the terminal width
// TODO: Add year headers to both outputs
// TODO: Write an actual stylesheet instead of using inline styles, add a flag for using an external stylesheet
// TODO: Reorder the years so the most recent is first, and add a flag for this

use argh::{self, FromArgs};
use chrono::{DateTime, Datelike, Utc};
use rayon::prelude::*;

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time;

static HTML_HEAD: &str =
    "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><title>Activity</title></head><body>";
static HTML_TAIL: &str = "</body>";

#[derive(FromArgs)]
#[allow(dead_code)]
/// Generates a nice activity graph from a bunch of Git repositories.
struct Args {
    /// whether to print verbose information.
    #[argh(switch, short = 'v')]
    verbose: bool,
    /// whether to print the resulting html into stdout. This is
    /// implied if an output file is not specified.
    #[argh(switch, short = 's')]
    stdout: bool,
    /// how many subdirectories deep the program should search (0
    /// (default) means as deep as the tree goes).
    #[argh(option, short = 'd', default = "0")]
    depth: i32,
    /// a regex for matching the author(s) whose commits are being
    /// counted.
    #[argh(option, short = 'a', default = "String::from(\"\")")]
    author: String,
    /// path to the directory (or directories) containing the
    /// repositories you want to include. Default is the current
    /// working directory.
    #[argh(option, short = 'r', default = "\"./\".into()")]
    repos: PathBuf,
    /// the file that the resulting html will be printed out to.
    #[argh(option, short = 'f')]
    file: Option<PathBuf>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ProjectMetadata {
    name: String,
    path: PathBuf,
}

fn main() {
    let start_time = time::Instant::now();
    let mut args: Args = argh::from_env();

    if !args.verbose && args.file.is_none() {
        args.stdout = true;
    }

    match fs::read_dir(&args.repos) {
        Ok(subdirs) => {
            // Find all the repository directories
            let depth = if args.depth == 0 {
                None
            } else {
                Some(args.depth)
            };
            let mut repos = HashSet::new();
            analyze_dirs(&mut repos, &args, &args.repos, subdirs, depth);
            if args.verbose {
                eprintln!("finished scanning for git repositories");
            }

            let commit_count = AtomicU32::new(0);
            let author = format!("--author={}", args.author);
            let mut commit_dates = repos
                .par_iter()
                // Collect the metadata of each commit in the repositories
                .map(|repo| {
                    let mut commit_dates: Vec<(DateTime<Utc>, &ProjectMetadata)> = Vec::new();
                    let path = &repo.path;
                    let commits = run_git(&path, &["log", "--all", "--format=oneline", &author]);
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
                            if args.verbose {
                                let count = commit_count.fetch_add(1, Ordering::Relaxed) + 1;
                                eprint!("commits accounted for {}\r", count);
                            }
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

            if args.verbose {
                eprintln!(
                    "counted up {} commits in {} repositories",
                    commit_dates.len(),
                    repos.len()
                );
            }

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
            let mut years =
                vec![vec![Vec::new(); weeks * 7]; (last_year - first_year + 1) as usize];

            // Organize the metadata so it's easy to go through when rendering the graphs
            let mut i = 0;
            for year in first_year..=last_year {
                // Loop through the years

                let days = &mut years[(year - first_year) as usize];
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
                        days[day_index].push(*metadata);
                    }

                    i += 1;
                }

                if args.verbose {
                    eprintln!(
                        "prepared year {} for rendering, {} commits processed so far",
                        year, i
                    );
                }
                if i >= commit_dates.len() {
                    break;
                }
            }

            let render_to_file_task = || {
                let mut output = args
                    .file
                    .as_ref()
                    .and_then(|path| File::create(path).ok())
                    .map(|file| BufWriter::new(file));
                if let Some(writer) = &mut output {
                    render_to_file(&args, &years, weeks, writer);
                }
            };

            let render_to_stdout_task = || {
                if args.stdout {
                    render_to_stdout(&args, &years, weeks);
                }
            };

            // Render to file and stdout at the same time, they don't
            // depend on each other.
            rayon::join(render_to_file_task, render_to_stdout_task);

            if args.verbose {
                eprintln!(
                    "finished all tasks, this run of the program took {:?}.",
                    time::Instant::now() - start_time
                );
            }
        }
        Err(err) => eprintln!("error: cannot read directory ({})", err),
    }
}

fn get_max_count(year: &[Vec<&ProjectMetadata>]) -> f32 {
    year.iter()
        .map(|metadata| metadata.len())
        .max()
        .unwrap_or(0)
        .max(1) as f32
}

fn render_to_file(
    args: &Args,
    years: &[Vec<Vec<&ProjectMetadata>>],
    weeks: usize,
    writer: &mut BufWriter<File>,
) {
    if args.verbose {
        eprint!("writing html to file...\r");
    }
    let _ = writeln!(writer, "{}", HTML_HEAD);
    for year in years {
        let max_count = get_max_count(year);
        let _ = writeln!(
            writer,
            "<table style='padding: 1em; width: auto; margin: auto;'><tbody>"
        );
        for day in 0..7 {
            let _ = writeln!(writer, "<tr>");
            for week in 0..weeks {
                let metadata = &year[day * weeks + week];
                let shade = ((metadata.len() as f32 / max_count) * 255.999) as i32;
                let _ = writeln!(
                    writer,
                    "<td style='width: 0.5em; height: 0.5em; background-color:rgb({},{},{}); border: solid #BBB 1px'></td>",
                    255,
                    255 - shade / 2,
                    255 - shade,
                );
            }
            let _ = writeln!(writer, "</tr>");
        }
        let _ = writeln!(writer, "</table></tbody>");
    }
    let _ = writeln!(writer, "{}", HTML_TAIL);
    if args.verbose {
        eprintln!("wrote html to file");
    }
}

fn render_to_stdout(args: &Args, years: &[Vec<Vec<&ProjectMetadata>>], weeks: usize) {
    if args.verbose {
        eprint!("writing ascii representation to stdout\r");
    }
    for year in years {
        let max_count = get_max_count(year);
        println!("");
        for day in 0..7 {
            for week in 0..weeks {
                let metadata = &year[day * weeks + week];
                let shade = metadata.len() as f32 / max_count;
                print!("{}", get_shaded_char(shade));
            }
            println!("");
        }
    }
    if args.verbose {
        eprintln!("wrote ascii representation to stdout");
    }
}

fn get_shaded_char(shade: f32) -> char {
    match shade {
        x if x > 0.5 => '\u{2593}',
        x if x > 0.0 => '\u{2592}',
        _ => '\u{2591}',
    }
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

fn analyze_dirs(
    git_paths: &mut HashSet<ProjectMetadata>,
    args: &Args,
    path: &Path,
    dirs: fs::ReadDir,
    depth: Option<i32>,
) {
    if args.verbose {
        if let Some(path) = path_to_string(&path) {
            eprint!("scanning: {}\r", path);
        }
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
                analyze_dirs(git_paths, args, &path, dirs, depth.map(|depth| depth - 1));
            }
        }
    }
}
