//! A bounded Spotify station preview for Home. The private Web API app
//! cannot call Recommendations, so this uses the same reader as song radio.

use super::*;

fn seed_track(tracks: &[Track]) -> Option<&Track> {
    tracks.iter().find(|track| {
        !track.is_local
            && track.is_playable != Some(false)
            && track.uri.starts_with("spotify:track:")
            && util::station_uri(&track.uri).is_some()
    })
}

impl App {
    pub(super) fn load_home_recommendations(&mut self) {
        if !self.home.requested
            || !self.settings.home.recommendations.visible
            || self.home.recommendations_generation == self.home.generation
        {
            return;
        }
        if !self.music_session_ready {
            if self.home.recommendations.get().is_none() {
                self.home.recommendations = Loadable::NotLoaded;
            }
            return;
        }
        let seed = match &self.home.top_tracks {
            Loadable::Loaded(tracks) => {
                seed_track(tracks).map(|track| (track.uri.clone(), track.name.clone()))
            }
            Loadable::Failed(_) => {
                self.home.recommendations_generation = self.home.generation;
                self.home.recommendations_loading = false;
                let error = "Couldn't load your listening history. Try again.".to_string();
                self.home
                    .recommendations
                    .refresh(Err::<Vec<Track>, _>(&error));
                self.home.recommendations_error = Some(error);
                return;
            }
            Loadable::NotLoaded | Loadable::Loading => {
                if self.home.recommendations.get().is_none() {
                    self.home.recommendations = Loadable::Loading;
                }
                return;
            }
        };
        self.home.recommendations_generation = self.home.generation;
        self.home.recommendations_error = None;
        let Some((seed, name)) = seed else {
            self.home.recommendations = Loadable::Loaded(Vec::new());
            self.home.recommendations_loading = false;
            self.home.recommendations_seed = None;
            self.home.recommendations_pending_seed = None;
            return;
        };
        self.load_generation = self.load_generation.wrapping_add(1);
        self.home.recommendations_request = self.load_generation;
        self.home.recommendations_loading = true;
        self.home.recommendations_pending_seed = Some(name);
        if self.home.recommendations.get().is_none() {
            self.home.recommendations = Loadable::Loading;
        }
        self.backend
            .home_recommendations(seed, self.home.recommendations_request);
    }

    pub(super) fn receive_home_recommendations(
        &mut self,
        generation: u64,
        result: Result<Vec<Track>, String>,
    ) {
        if generation != self.home.recommendations_request
            || self.home.recommendations_generation != self.home.generation
            || !self.home.recommendations_loading
        {
            return;
        }
        self.home.recommendations_loading = false;
        self.home.recommendations_error = result.as_ref().err().cloned();
        if let Ok(tracks) = &result {
            self.home.recommendations_seed = self.home.recommendations_pending_seed.take();
            let uris = tracks.iter().map(|track| track.uri.clone()).collect();
            self.request_contains(uris);
        } else {
            self.home.recommendations_pending_seed = None;
        }
        // A refresh failure leaves the previous songs in place, with Retry beside it.
        self.home.recommendations.refresh(result);
    }

    pub(super) fn retry_home_recommendations(&mut self) {
        if !self.network_reachable
            || self.home.recommendations_loading
            || !self.settings.home.recommendations.visible
            || !self.music_session_ready
        {
            return;
        }
        self.home.recommendations_generation = self.home.generation.wrapping_sub(1);
        self.home.recommendations_error = None;
        let needs_history = match &self.home.top_tracks {
            Loadable::Failed(_) | Loadable::NotLoaded => true,
            Loadable::Loaded(tracks) => seed_track(tracks).is_none(),
            Loadable::Loading => return,
        };
        if needs_history {
            self.home.top_tracks = Loadable::Loading;
            if self.home.recommendations.get().is_none() {
                self.home.recommendations = Loadable::Loading;
            }
            self.backend.api(ApiRequest::TopTracks {
                offset: 0,
                full: false,
                generation: self.home.generation,
            });
        } else {
            self.load_home_recommendations();
        }
    }

    pub(super) fn home_music_session_changed(&mut self) {
        if self.music_session_ready {
            self.load_home_recommendations();
        } else if self.home.recommendations_loading {
            self.home.recommendations_loading = false;
            self.home.recommendations_generation = self.home.generation.wrapping_sub(1);
            self.home.recommendations_pending_seed = None;
            let error = "Connect Spotify music access to load your listening mix.".to_string();
            self.home
                .recommendations
                .refresh(Err::<Vec<Track>, _>(&error));
            self.home.recommendations_error = Some(error);
        }
    }
}
