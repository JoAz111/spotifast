#!/usr/bin/env python3
"""Profile the actual folder-count helpers against a named Git baseline.

This isolates counting, not complete UI frames, Spotify loading or playback.
Requires rustc on PATH. Generated files stay in ignored .cache/.
"""
import argparse
import os
from pathlib import Path
import shlex
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def function(source, name):
    start = source.index(f"fn {name}(")
    body = source.index("{", start)
    depth = 1
    end = body + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", default="83c273e388bcbe1c56202dd36c8a2fe9b41dec77")
    args = parser.parse_args()
    baseline = subprocess.check_output(
        ["git", "show", f"{args.baseline}:src/ui/sidebar.rs"], cwd=ROOT, text=True
    )
    candidate = (ROOT / "src/ui/sidebar.rs").read_text()
    source = '''
mod player {
    pub enum RootlistEntry {
        Playlist(String),
        FolderStart { id: String, name: String },
        FolderEnd,
    }
}
'''
    source += function(baseline, "folder_playlists") + "\n"
    source += function(candidate, "folder_playlist_counts") + "\n"
    source += '''
fn main() {
    use player::RootlistEntry::*;
    use std::hint::black_box;
    use std::time::Instant;
    for size in [512, 4096] {
        let rows: Vec<_> = (0..size).flat_map(|i| [
            FolderStart { id: i.to_string(), name: String::new() },
            Playlist(format!("spotify:playlist:{i}")), FolderEnd,
        ]).collect();
        // Both operations consume the same synthetic rootlist and count every
        // sibling folder. Id lookup and allocation remain inside the timing.
        let mut old = Vec::new();
        let mut new = Vec::new();
        for _ in 0..9 {
            let start = Instant::now();
            let sum: usize = rows.iter().filter_map(|row| match row {
                FolderStart { id, .. } => Some(folder_playlists(black_box(&rows), id)),
                _ => None,
            }).sum();
            black_box(sum);
            assert_eq!(sum, size);
            old.push(start.elapsed().as_nanos());
            let start = Instant::now();
            let counts = folder_playlist_counts(black_box(&rows));
            black_box(&counts);
            assert_eq!(counts.len(), size);
            assert!(counts.values().all(|&count| count == 1));
            new.push(start.elapsed().as_nanos());
        }
        old.sort(); new.sort();
        println!("folders={size} rows={} baseline_median_ns={} candidate_median_ns={}", rows.len(), old[4], new[4]);
    }
}
'''
    scratch = ROOT / ".cache/folder-count-profile"
    scratch.mkdir(parents=True, exist_ok=True)
    path = scratch / "fixture.rs"
    path.write_text(source)
    for label, optimize in [("debug", "0"), ("optimized", "3")]:
        binary = scratch / label
        command = [os.environ.get("RUSTC", "rustc"), "--edition=2024", "-C", f"opt-level={optimize}"]
        command += shlex.split(os.environ.get("RUSTFLAGS", ""))
        subprocess.run(command + [str(path), "-o", str(binary)], check=True, cwd=ROOT)
        binary.chmod(0o755)
        print(label, flush=True)
        subprocess.run([str(binary)], check=True)


if __name__ == "__main__":
    main()
