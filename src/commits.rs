use chrono::{DateTime, Utc};
use rayon::prelude::*;

use std::collections::HashSet;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::{log, ProjectMetadata};

pub fn find_dates<'a>(
    author: Option<&String>,
    repos: &'a HashSet<ProjectMetadata>,
) -> Vec<(DateTime<Utc>, &'a ProjectMetadata)> {
    let commit_count = AtomicU32::new(0);
    let author_flag = author.as_ref().map(|author| format!("--author={}", author));
    let commit_dates = repos
        .par_iter()
        // Collect the metadata of each commit in the repositories
        .map(|repo| {
            let mut commit_dates: Vec<(DateTime<Utc>, &ProjectMetadata)> = Vec::new();
            let path = &repo.path;
            // TODO: Read dates from the log to avoid most of the current git cmds
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
                    log::verbose_println(&format!("commits accounted for {}\r", count), true);
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

    log::verbose_println(
        &format!(
            "counted up {} commits in {} repositories",
            commit_dates.len(),
            repos.len()
        ),
        false,
    );

    commit_dates
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
