use humansize::DECIMAL;
use std::ffi::OsString;
use std::fs::{read_dir, read_link};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};
use walkdir::WalkDir;

use clap::Parser;

/*
Process:
* Start in the root directory
* Recursively iterate through directories, if they contain both a Cargo.toml and a target/ directory,
  * Check the modification date of the most recently modified file in the folder, if older than a certain number of days, delete the target directory
 */

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path of the folder to clean
    #[arg(short, long)]
    path: PathBuf,
    /// Minimum number of days since modification to be cleaned
    #[arg(short, long)]
    days_old: usize,
    /// Whether or not it should actually be deleted
    #[arg(long, default_value_t = false)]
    actually_delete: bool,
}
/// Returns true if target dir should be deleted
pub fn check_target_dir_date(dir: &Path, cutoff: SystemTime) -> Option<u64> {
    let mut total_size = 0;
    for entry in WalkDir::new(dir) {
        match entry {
            Ok(entry) => {
                let a = 0;
                match entry.metadata() {
                    Ok(metadata) => {
                        match metadata.modified() {
                            Ok(time) => {
                                if time > cutoff {
                                    return None;
                                }
                            }
                            Err(e) => {
                                if e.kind() == ErrorKind::Unsupported {
                                    println!("This platform does not support finding the modification date of files!");
                                    std::process::exit(1);
                                }
                            }
                        }
                        if metadata.is_file() {
                            total_size += metadata.len();
                        }
                    }
                    Err(e) => {
                        let io_error = e.io_error();
                        if io_error.is_some() && io_error.unwrap().kind() == ErrorKind::Unsupported
                        {
                            println!("This platform does not support finding the metadata date of files!");
                            std::process::exit(1);
                        }
                        println!(
                            "Error accessing metadata of file {}: {e}, skipping cleaning folder {}",
                            entry.path().display(),
                            dir.display()
                        );
                        return None;
                    }
                }
            }
            Err(e) => println!("Error accessing entry in folder: {e}"),
        }
    }
    Some(total_size)
}
pub fn scan_for_target_dirs(
    dir: PathBuf,
    cutoff: Option<SystemTime>,
    actually_delete: bool,
    stack: &mut Vec<PathBuf>,
) -> u64 {
    let mut to_check = Vec::new();
    let mut has_cargo_toml = false;
    let mut has_target_dir = false;
    match read_dir(&dir) {
        Ok(entries) => {
            for entry in entries {
                match entry {
                    Ok(entry) => {
                        if let Ok(name) = entry.file_name().into_string() {
                            if name.as_str() == "Cargo.toml" {
                                has_cargo_toml = true
                            } else if name.as_str() == "target" {
                                has_target_dir = true
                            }
                            if has_cargo_toml && has_target_dir {
                                break;
                            }
                        }
                        let file_type = entry.file_type().unwrap();
                        let path = entry.path();
                        if file_type.is_dir() {
                            to_check.push(path);
                        } else if file_type.is_symlink() {
                            match read_link(&path) {
                                Ok(inner) => {
                                    let symlink_target = if inner.is_relative() {
                                        dir.join(&inner)
                                    } else {
                                        inner
                                    };
                                    match std::fs::metadata(&symlink_target) {
                                        Ok(metadata) => {
                                            if metadata.is_dir() {
                                                to_check.push(path);
                                            }
                                        }
                                        Err(e) => println!(
                                        "Error reading metadata of entry {} behind symlink {}: {}",
                                        symlink_target.display(),
                                        path.display(),
                                        e
                                    ),
                                    }
                                }
                                Err(e) => {
                                    println!("Error following symlink {}: {}", path.display(), e)
                                }
                            }
                        }
                    }
                    Err(e) => println!("Error accessing entry in folder: {e}"),
                }
            }
        }
        Err(e) => println!("Error scanning directory {}: {}", dir.display(), e),
    }
    if has_cargo_toml && has_target_dir {
        let target_path = dir.join("target");
        let should_delete = if let Some(cutoff) = cutoff {
            check_target_dir_date(&target_path, cutoff)
        } else {
            match fs_extra::dir::get_size(&target_path) {
                Ok(size) => Some(size),
                Err(e) => {
                    println!("");
                    return 0;
                }
            }
        };
        if let Some(size) = should_delete {
            println!(
                "Deleting {} of files in target directory {}",
                humansize::format_size(size, DECIMAL),
                target_path.display()
            );
            if actually_delete {
                if let Err(e) = std::fs::remove_dir_all(&target_path) {
                    println!(
                        "Error deleting target directory {}: {}",
                        target_path.display(),
                        e
                    );
                }
            }
            return size;
        } else {
            return 0;
        }
    } else {
        let mut total_size = 0;
        'a: for thing in to_check {
            let canonical_path = match thing.clone().canonicalize() {
                Ok(v) => v,
                Err(e) => {
                    println!("Error resolving path {}: {}", thing.display(), e);
                    continue 'a;
                }
            };
            /*println!("Thing: {}", thing.display());
            println!("Canonical: {}", canonical_path.display());
            println!(
                "Stack: {}",
                stack
                    .iter()
                    .map(|s| s.to_str().unwrap())
                    .collect::<Vec<_>>()
                    .join(", ")
            );*/
            for i in 0..stack.len() {
                if stack[i] == canonical_path {
                    if stack.contains(&canonical_path) {
                        println!("Warning: circular symlink reference detected:");
                        for j in i..stack.len() {
                            println!("\t{}", stack[j].display());
                        }
                        println!("\t{}", canonical_path.display());
                        continue 'a;
                    }
                    break;
                }
            }
            stack.push(canonical_path);
            total_size += scan_for_target_dirs(thing, cutoff, actually_delete, stack);
            stack.pop();
        }
        return total_size;
    }
}
fn main() {
    let args = Args::parse();
    let mut stack = Vec::new();
    let cutoff = if args.days_old == 0 {
        None
    } else {
        Some(SystemTime::now() - std::time::Duration::from_secs((3600 * 24 * args.days_old) as u64))
    };
    println!("WARNING: recursive symlinks WILL cause this program to freeze.");
    if !args.actually_delete {
        println!("Because you ran without --actually-delete, no folders will actually be deleted. This will simply list out what would be deleted, which is useful for debug purposes.");
    }
    stack.push(args.path.clone());
    let start_time = Instant::now();
    let size = scan_for_target_dirs(args.path, cutoff, args.actually_delete, &mut stack);
    println!(
        "Deleted {} of data in target folders in {} seconds",
        humansize::format_size(size, DECIMAL),
        (Instant::now() - start_time).as_secs_f32(),
    );
}
