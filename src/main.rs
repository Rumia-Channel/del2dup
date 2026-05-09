use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use clap::Parser;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

#[derive(Parser, Debug)]
#[command(name = "del2dup")]
struct Args {
    #[arg(help = "Folders to search for duplicate files")]
    folders: Vec<PathBuf>,

    #[arg(
        short = 'e',
        long = "ext",
        value_name = "EXT",
        help = "File extensions to include (without leading dot)"
    )]
    extensions: Vec<String>,

    #[arg(long, help = "Actually delete duplicate files")]
    delete: bool,
}

struct FileInfo {
    path: PathBuf,
    folder_index: usize,
    depth: usize,
}

fn sha256_hex(path: &Path) -> String {
    let mut file = fs::File::open(path).unwrap_or_else(|e| {
        panic!("Cannot open {}: {}", path.display(), e);
    });
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(e) => panic!("Error reading {}: {}", path.display(), e),
        }
    }
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

fn depth_from(base: &Path, target: &Path) -> usize {
    target
        .strip_prefix(base)
        .unwrap_or(target)
        .components()
        .count()
}

fn main() {
    let args = Args::parse();

    if args.folders.is_empty() || args.extensions.is_empty() {
        let mut cmd = <Args as clap::CommandFactory>::command();
        cmd.print_help().unwrap();
        return;
    }

    let exts: Vec<String> = args
        .extensions
        .iter()
        .map(|e| e.trim_start_matches('.').to_lowercase())
        .collect();

    let mut seen_paths: HashMap<PathBuf, usize> = HashMap::new();
    let mut files: Vec<FileInfo> = Vec::new();

    for (fi, folder) in args.folders.iter().enumerate() {
        let can_folder = folder.canonicalize().unwrap_or_else(|_| folder.clone());

        eprint!("\rScanning \"{}\"...", folder.display());
        io::stderr().flush().unwrap();

        for entry in WalkDir::new(&can_folder)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();

            let ext_match = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| exts.contains(&e.to_lowercase()))
                .unwrap_or(false);
            if !ext_match {
                continue;
            }

            let can_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            if seen_paths.contains_key(&can_path) {
                continue;
            }

            let depth = depth_from(&can_folder, path);
            let idx = files.len();
            files.push(FileInfo {
                path: can_path.clone(),
                folder_index: fi,
                depth,
            });
            seen_paths.insert(can_path, idx);
        }
    }
    eprint!("\r\x1b[2K"); // clear line
    eprintln!("Scanned {} folder(s), found {} file(s).", args.folders.len(), files.len());

    let mut size_groups: HashMap<u64, Vec<usize>> = HashMap::new();
    for (i, info) in files.iter().enumerate() {
        let size = fs::metadata(&info.path).map(|m| m.len()).unwrap_or(0);
        size_groups.entry(size).or_default().push(i);
    }

    let mut hash_groups: HashMap<String, Vec<usize>> = HashMap::new();
    let candidates: Vec<usize> = size_groups
        .values()
        .filter(|v| v.len() >= 2)
        .flatten()
        .copied()
        .collect();
    let total_candidates = candidates.len();

    if total_candidates > 0 {
        eprintln!(
            "Hashing {} file(s) across {} size group(s)...",
            total_candidates,
            size_groups.values().filter(|v| v.len() >= 2).count()
        );
        for (done, &i) in candidates.iter().enumerate() {
            if (done + 1) % 100 == 0 || done + 1 == total_candidates {
                eprint!("\r  {}/{}", done + 1, total_candidates);
                io::stderr().flush().unwrap();
            }
            let hash = sha256_hex(&files[i].path);
            hash_groups.entry(hash).or_default().push(i);
        }
        eprintln!();
    }

    let mut total_deleted = 0u64;
    let mut total_kept = 0u64;

    for (_, indices) in &hash_groups {
        if indices.len() <= 1 {
            continue;
        }

        let keep_idx = indices
            .iter()
            .max_by_key(|&&i| (files[i].depth, -(files[i].folder_index as isize)))
            .unwrap();

        println!("Duplicate group ({} files):", indices.len());
        for &i in indices {
            let path = &files[i].path;
            if i == *keep_idx {
                println!("  [KEEP]  {}", path.display());
                total_kept += 1;
            } else if args.delete {
                match fs::remove_file(path) {
                    Ok(()) => {
                        println!("  [DEL]   {}", path.display());
                        total_deleted += 1;
                    }
                    Err(e) => eprintln!("  [FAIL]  {} ({})", path.display(), e),
                }
            } else {
                println!("  [DEL?]  {}", path.display());
                total_deleted += 1;
            }
        }
    }

    if total_deleted > 0 {
        if args.delete {
            println!(
                "\nDone. Kept {} files, deleted {} duplicates.",
                total_kept, total_deleted
            );
        } else {
            println!(
                "\nDry run. Kept {} files, {} would be deleted. Use --delete to actually delete.",
                total_kept, total_deleted
            );
        }
    } else {
        println!("\nNo duplicate files found.");
    }
}
