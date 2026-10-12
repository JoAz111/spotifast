//! Lyrics from Spotify, with a local disk cache. No third-party lookup.
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;
const CACHE_LIFETIME: Duration = Duration::from_secs(30 * 24 * 60 * 60);
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    /// When the line starts; `None` in lyrics that carry no timing.
    pub at_ms: Option<u32>,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lyrics {
    pub lines: Vec<Line>,
    /// Every line has a time, so the panel can follow the song.
    pub synced: bool,
    /// The database knows there are no words.
    pub instrumental: bool,
}

impl Lyrics {
    /// The line being sung at `position_ms`, or `None` before the first one
    /// starts and for lyrics without timing.
    pub fn active_line(&self, position_ms: u32) -> Option<usize> {
        if !self.synced {
            return None;
        }
        let started = self
            .lines
            .partition_point(|line| line.at_ms.is_some_and(|at| at <= position_ms));
        started.checked_sub(1)
    }
}

/// Parses Spotify's `color-lyrics` response.
pub fn from_spotify(json: &serde_json::Value) -> Option<Lyrics> {
    let lyrics = json.get("lyrics")?;
    let synced = lyrics.get("syncType").and_then(|value| value.as_str()) == Some("LINE_SYNCED");
    let lines: Vec<Line> = lyrics
        .get("lines")?
        .as_array()?
        .iter()
        .filter_map(|line| {
            let text = line.get("words")?.as_str()?.trim();
            if text.is_empty() || text == "\u{266a}" {
                return None;
            }
            let at_ms = line
                .get("startTimeMs")
                .and_then(|value| {
                    value
                        .as_str()
                        .and_then(|text| text.parse().ok())
                        .or_else(|| value.as_u64().and_then(|n| u32::try_from(n).ok()))
                })
                .filter(|_| synced);
            Some(Line {
                at_ms,
                text: text.to_string(),
            })
        })
        .collect();
    if lines.is_empty() {
        return None;
    }
    let synced = synced && lines.iter().all(|line| line.at_ms.is_some());
    Some(Lyrics {
        lines,
        synced,
        instrumental: false,
    })
}

/// The cached answer at `path`, while it is fresh.
pub fn cached(path: &Path) -> Option<Option<Lyrics>> {
    read_cache(path)
}

/// Remember an answer at `path`, `None` included.
pub fn store(path: &Path, found: &Option<Lyrics>) {
    write_cache(path, found);
}

#[derive(Serialize, Deserialize)]
struct Cached {
    found: Option<Lyrics>,
}

fn read_cache(path: &Path) -> Option<Option<Lyrics>> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    if modified.elapsed().unwrap_or(CACHE_LIFETIME) >= CACHE_LIFETIME {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<Cached>(&text)
        .ok()
        .map(|cached| cached.found)
}

fn write_cache(path: &Path, found: &Option<Lyrics>) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string(&Cached {
        found: found.clone(),
    }) {
        let _ = std::fs::write(path, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spotify_lines_follow_time_and_ignore_empty_words() {
        let json = serde_json::json!({"lyrics": {"syncType": "LINE_SYNCED", "lines": [
            {"startTimeMs": "5000", "words": "First"},
            {"startTimeMs": "10000", "words": "Second"},
            {"startTimeMs": "15000", "words": "♪"}
        ]}});
        let lyrics = from_spotify(&json).unwrap();
        assert!(lyrics.synced);
        assert_eq!(lyrics.lines.len(), 2);
        assert_eq!(lyrics.active_line(0), None);
        assert_eq!(lyrics.active_line(5000), Some(0));
        assert_eq!(lyrics.active_line(12000), Some(1));
    }
    #[test]
    fn incomplete_spotify_timing_does_not_scroll_to_the_wrong_line() {
        let lyrics = from_spotify(&serde_json::json!({"lyrics": {
            "syncType": "LINE_SYNCED", "lines": [{"words": "No timestamp"}]
        }}))
        .unwrap();
        assert!(!lyrics.synced);
        assert_eq!(lyrics.active_line(5000), None);
        assert!(from_spotify(&serde_json::json!({"lyrics": {"lines": []}})).is_none());
    }
    #[test]
    fn cache_distinguishes_not_loaded_from_no_lyrics() {
        let dir =
            std::env::temp_dir().join(format!("spotifast-lyrics-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("lyrics.json");
        assert_eq!(cached(&path), None);
        store(&path, &None);
        assert_eq!(cached(&path), Some(None));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
