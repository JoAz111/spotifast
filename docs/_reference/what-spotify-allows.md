---
title: What Spotify Lets a Client Do
description: What Spotify's Web API and librespot provide, and what neither supports.
nav_order: 3
---

Spotifast uses Spotify's
[Web API](https://developer.spotify.com/documentation/web-api) for account and
catalogue data. It uses [librespot](https://github.com/librespot-org/librespot)
for extra metadata. On this Mac, the official, unmodified
[Soloist](https://developer.spotify.com/documentation/soloist) engine owns audio
playback through an experimental Apple Silicon compatibility runtime. Other
platforms retain the existing librespot player.

Features missing from both cannot be added to Spotifast. They may become
possible if Spotify adds an API or librespot adds lawful support.

## Web API

Spotifast uses the Web API for:

- **Account:** profile details, followed artists, top artists and tracks, and
  the last fifty plays. Spotifast keeps a longer local
  [history](/using-spotifast/#recent).
- **Library:** playlists, saved tracks, albums, shows, and episodes. It can
  also save and remove items.
- **Playlists:** reading, creating, renaming, changing the description and
  visibility, adding and removing songs, reordering songs, and following and
  unfollowing. Custom playlist cover uploads are available since 0.9.0.
- **Catalogue:** albums, artists, tracks, shows, episodes and search, where
  permitted for the user's development-mode registration. Some artist endpoints
  remain restricted.
- **Playback control:** listing devices, transferring playback, play, pause,
  next, previous, seek, shuffle, repeat, volume, and reading or adding to the
  queue.

This fork uses only the user's private Web API registration. Spotify's
[July 2026 quota policy](https://developer.spotify.com/blog/2026-07-23-web-api-quota-updates)
counts development-mode quota per developer account across all its Client IDs.
Spotifast paces requests and preserves the full `Retry-After` cooldown.

Spotify's [development-mode restrictions](https://developer.spotify.com/blog/2024-11-27-changes-to-the-web-api)
exclude the Recommendations endpoint, related artists, audio features and some
Spotify-owned playlists. The Home listening mix instead uses a bounded Spotify
song-radio preview through the metadata session. It is labelled "Inspired by
your listening" and identifies its seed. It is not Spotify's complete Home
recommendation algorithm. Unavailable music access, empty history and failures
settle visibly, with a retry limited to the affected shelf.

Supported library, search and playback surfaces include podcasts and episodes.
Legacy saved `music_only` settings are ignored, so the interface does not hide
them. Audiobooks remain excluded from the Podcasts shelf.

## librespot session

librespot signs in to Spotify and uses the same protocol as Spotify's own
clients. Spotifast uses its session for:

- **Playlist folders and order.** Spotifast can read them from Spotify's
  rootlist. librespot cannot create, rename, or move folders.
- **Playlist permissions.** The rootlist shows when a playlist shared by
  invitation can be edited. The Web API's `collaborative` flag does not cover
  these playlists. Spotifast cannot manage collaborators.
- **Playlist metadata and songs.** The session can read playlists the
  development-mode Web API cannot serve, without a shared app registration. Whether a
  playlist is public, the owner's name when the session gives none, and the
  mosaic of a playlist without a cover of its own, come from the Web API's
  library list. These rows carry
  no per-market availability flag. A song the Web API had already greyed
  out, on an earlier page or in the cache of an earlier visit, stays greyed
  out when the session reads the rows. This knowledge belongs to the signed-in
  account, and a newer Web API answer takes precedence over an older disk
  cache. A song the session reads first has unknown availability; if it cannot
  play, it is skipped when reached, as it would be anywhere else.
- **Library playlists the Web API lists blank.** The library list gives
  some of Spotify's own playlists, "This Is" ones among them, no name,
  cover, description, or songs. Spotifast reads each one's header over the
  session once per sign-in to fill its row. Without music metadata access those
  rows may remain incomplete.
- **Lyrics** when Spotify has them.
- **Display names** for the user IDs attached to songs in a playlist.
- **Precise EP types** for releases that the Web API groups with singles.
- **Radio and autoplay** through Spotify's context resolver: stations seeded
  by a song, playlist, album, or artist. Each resolution is a fresh mix of 50
  songs, so a radio page plays the songs it shows rather than asking again.
- **Audiobook detection** for saved shows, which the Web API lists as podcasts.

## librespot playback

librespot provides:

- Spotify catalogue playback at up to 320 kbps.
- Gapless playback, normalisation, and a local audio cache.
- Spotify Connect, so another Spotify client can transfer playback to this
  computer.
- Shuffle, repeat, seek, and volume.
- Control of restricted Connect devices, including Sonos, through the signed-in
  playback session. Sonos receivers are discovered on the local network;
  the Web API alone cannot start or control them.
- Songs and podcast episodes.

Spotify Premium is required. librespot cannot play audio with a free account.

Spotifast uses a small librespot fork. Its patches add queue controls,
an event for rejected audio keys and metadata integration. The removed visualizer
surfaces are not part of this fork.
They are listed in `Cargo.toml`. Larger changes go upstream first.

## Not available

The Web API and librespot do not provide these features:

- **Pins shared with the Spotify app.** Spotify stores pins in its private
  `your-library` service. There is no Web API for it, and librespot does not
  support its protocol. Spotifast's pins are local and are stored in
  `settings.json`. See [issue #31](https://github.com/crmne/spotifast/issues/31).
- **Editing playlist folders.** librespot can only read them.
- **Smart Shuffle, Jam, Blend, and similar Spotify features.** Spotify
  generates these for its own clients. Spotifast only has plain shuffle.
  A Blend that a Spotify app already made opens like any playlist, with
  who added each song, but Spotifast cannot start or manage one.
- **Lossless through librespot.** librespot does not receive lossless streams.
  This Mac integration instead uses Spotify's official Soloist engine, which
  has decoded FLAC at 44.1 kHz during authenticated playback. That result does
  not establish source bit depth or bit-perfect hardware output. Soloist's local
  WebSocket state does not report current-track codec or quality; a preference
  for lossless alone is insufficient evidence for a current-track badge.
- **Local files.** librespot only streams Spotify's catalogue. It cannot fetch
  audio for a `spotify:local:` entry. Playing files from disk would require a
  separate player. See [issue #3](https://github.com/crmne/spotifast/issues/3).
- **Audiobooks.** librespot does not play them. Spotify lists some
  audiobooks among saved shows; since 0.10.0, Spotifast asks the
  librespot session which ones and leaves them out of the Podcasts shelf.
- **Google Cast.** Spotify's own apps find Cast speakers on the local
  network and start Spotify's receiver on them. librespot has no Cast
  sender, and the Web API only lists a Cast device once a Spotify app has
  already woken it, so Spotifast cannot discover or start one itself. See
  [issue #566](https://github.com/crmne/spotifast/issues/566).
- **Offline listening and downloads.** Spotify's DRM and the project's scope
  rule these out.
- **Playback speed and crossfade.** librespot supports neither. Spotifast
  would have to add them to its own audio path.
- **Free-account playback.** Replacing Spotify audio with another source is
  also out of scope. See the
  [contribution guide](https://github.com/crmne/spotifast/blob/main/CONTRIBUTING.md).
- **Friend activity, private-session status, and similar social features.**
  Spotify has no public API for them.
- **Canvas videos and video podcasts.** librespot does not provide them.
- **Play counts.** Spotify shows them only through a private endpoint its own
  apps use. The Web API has no play counts, and librespot does not provide
  them. See [issue #543](https://github.com/crmne/spotifast/issues/543).
