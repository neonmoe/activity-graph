use chrono::{DateTime, Utc};
#[cfg(feature = "rayon")]
use rayon::prelude::*;

use std::collections::HashSet;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::{log, ProjectMetadata};

pub fn find_dates(
    author: Option<&String>,
    repos: &HashSet<ProjectMetadata>,
) -> Vec<(DateTime<Utc>, ProjectMetadata)> {
    let commit_count = AtomicU32::new(0);
    let author_flag = author.as_ref().map(|author| format!("--author={}", author));

    #[cfg(feature = "rayon")]
    let repo_iter = repos.par_iter();
    #[cfg(not(feature = "rayon"))]
    let repo_iter = repos.iter();

    let commit_dates = repo_iter.map(|repo| {
        let mut commit_dates: Vec<(DateTime<Utc>, ProjectMetadata)> = Vec::new();
        let path = &repo.path;
        let mut args = vec!["log", "--all", "--format=format:%ai", "--date=iso8601"];
        if let Some(author_flag) = &author_flag {
            args.push(author_flag);
        }
        let commits = run_git(&path, &args);
        for date in commits.lines().filter_map(|date| date.parse().ok()) {
            let count = commit_count.fetch_add(1, Ordering::Relaxed) + 1;
            log::verbose_println(&format!("commits accounted for {}\r", count), true);
            commit_dates.push((date, repo.clone()));
        }
        commit_dates
    });

    #[cfg(feature = "rayon")]
    let commit_dates = commit_dates.reduce(Vec::new, |mut a, b| {
        a.extend(b);
        a
    });
    #[cfg(not(feature = "rayon"))]
    let commit_dates = commit_dates.fold(Vec::new(), |mut a, b| {
        a.extend(b);
        a
    });

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
