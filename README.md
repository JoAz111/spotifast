# Spotifast Soloist

A native Spotify music client with a single dark interface. Browse your library,
search music, edit playlists, manage your queue, view lyrics, and control playback
from the window or your Mac's media keys.

This personal fork uses Spotify's official Soloist engine through an experimental
Apple Silicon compatibility runtime. Setup and qualification are documented in
[Soloist on macOS](docs/SOLOIST_MACOS.md). Spotify Premium and your own Soloist key
are required. The official engine is installed separately and is not included in
this repository or the app bundle.

Winamp layouts, skins, equalizers, visualizers, custom palettes and desktop theme
integrations have been removed. Older settings files remain readable; obsolete
appearance keys are ignored. Album artwork can still color music backgrounds.

The Spotify Web API registration is supplied locally at build time. Keep the
local identity file out of Git and never put a Spotify client secret in a native
app. See the build instructions in [Soloist on macOS](docs/SOLOIST_MACOS.md).

Built with Rust and egui, based on [Spotifast](https://github.com/crmne/spotifast).
The original code and this fork are MIT licensed; see [LICENSE](LICENSE).
