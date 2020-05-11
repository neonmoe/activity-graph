use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use crate::{log, ProjectMetadata};

pub fn from_paths(paths: &[PathBuf], depth: Option<i32>) -> HashSet<ProjectMetadata> {
    let repos = paths
        .iter()
        .map(|repo_dir| {
            match fs::read_dir(&repo_dir) {
                Ok(subdirs) => {
                    // Find all the repository directories
                    let mut repos = HashSet::new();
                    analyze_dir(&mut repos, &repo_dir, subdirs, depth);
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
    log::verbose_println("finished scanning for git repositories", false);
    repos
}

fn analyze_dir(
    git_paths: &mut HashSet<ProjectMetadata>,
    path: &Path,
    dirs: fs::ReadDir,
    depth: Option<i32>,
) {
    if let Some(path) = path
        .canonicalize()
        .ok()
        .map(PathBuf::into_os_string)
        .and_then(|path| path.into_string().ok())
    {
        log::verbose_println(&format!("scanning: {}\r", path), true);
    }

    let dirs: Vec<fs::DirEntry> = dirs.filter_map(Result::ok).collect();
    if dirs
        .iter()
        .map(fs::DirEntry::file_name)
        .any(|file_name| file_name == ".git")
    {
        if let Some(name) = path
            .file_name()
            .and_then(OsStr::to_str)
            .map(ToString::to_string)
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
                analyze_dir(git_paths, &path, dirs, depth.map(|depth| depth - 1));
            }
        }
    }
}
