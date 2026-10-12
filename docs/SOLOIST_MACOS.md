# Soloist on macOS

This fork uses the unchanged official Linux ARM64 Soloist engine on Apple Silicon through the community elfuse macOS compatibility runtime. It is an experimental personal test build. Spotify does not publish a native macOS Soloist engine. The compatibility runtime is not a security sandbox and has the invoking user's host file and network authority.

The native Rust interface remains Spotifast. With `SPOTIFAST_SOLOIST_SOCKET` set by the launcher, all local audio comes from Soloist. The optional librespot session supplies playlist folders, playlist metadata, lyrics and radio; it creates no player, audio output or second Spotify Connect receiver. Web API sign-in remains separate.

## Music features

The main interface defaults to music: Home, songs, artists, albums, playlists, Liked Songs, queue and lyrics. Podcast and episode navigation, shelves and search results are hidden. Hidden Home podcast requests also stop. Set `music_only` to `false` in settings to restore the optional upstream surfaces. Direct links and episodes already present in mixed playlists remain internally supported.

Local controls support play/pause, next/previous, seek, volume, ordinary shuffle, repeat, queue additions and Connect activation. Album/playlist playback retains the selected row, full context, filtered track list, duplicates and position by activating Soloist first, then using Spotify's Web API. Explicit shuffle and repeat choices are sent in sequence with that load. A failed or disconnected mutation is never automatically replayed.

Soloist's local protocol exposes neither the account ID nor the Connect device ID. Pair Soloist and sign into Spotifast with the **same Spotify account**. A private socket verifies the local OS process, not account equality. A device switch in another Spotify client can race a Web API load targeting the active device; this API limitation is not hidden by guessing a device name.

## Current limitations

- Premium and a personal Soloist API key are required. Keys are entered locally in a native secure field and stored in Keychain, never in chat or source.
- Official Soloist supports Premium lossless up to 24-bit/44.1 kHz. This build must still verify authenticated playback and the actual output format on the target Mac. A successful startup or float32 audio sink alone does not prove lossless or bit-perfect output.
- Queue clear, removal and reordering are disabled for Soloist because its documented local API does not provide them. Queue queries request at most 80 items. Unsolicited queue broadcasts can contain only 10; query and broadcast messages have no IDs, so a racing broadcast can be a current but shortened snapshot.
- Crossfade/Automix and other engine capabilities do not automatically become interface controls. Smart Shuffle, Jam, DJ, offline downloads and full Spotify feature parity are not implemented here.
- Spotifast's equalizer, audio-cache/bitrate controls and audio visualizers do not control external Soloist audio and are hidden in this mode.
- The compatibility runtime targets Apple Silicon. Intel Mac, Windows and Linux Soloist integration have not been qualified by this change.

## Local setup and lifecycle

The combined native launcher selects the official engine file stored separately from the app. The official binary is not bundled or committed. Use Spotify's documented download endpoint and renew expired builds according to its Soloist documentation.

The packaged runtime uses a dedicated `~/Library/Application Support/Spotifast Soloist` profile. The Rust app also uses its own `spotifast-soloist` profile so it does not migrate or overwrite the original Spotifast profile. The key and each Spotify grant are separate credentials.

Start the receiver, pair the named receiver using Spotify Connect, and sign into the music interface with that account. Optional **Playlist and lyrics access** in Settings opens the separate streaming-permission browser approval needed for internal metadata reads. Normal library and catalog access use the Web API grant.

Quitting the music interface stops its owned receiver. Spotifast sign-out cancels pending commands and requests an explicit pause; it does not revoke Soloist's independently paired session. The launcher's reset-pairing action clears only its initialized local receiver state and cache, retaining the API key. It does not revoke a cloud account grant.

## Control boundary

The raw Soloist WebSocket API has no client authentication. This runtime remaps only its selected guest control listener to a private Unix socket: parent directory mode 0700, socket 0600. The Rust client checks the owner, permissions, peer UID and socket identity, bounds messages and command admission, and discards old account/connection operations. There is no TCP fallback for that API.

Spotify Connect discovery and pairing are separate network services. macOS can therefore ask whether `elfuse` may accept incoming connections. That prompt is not evidence of a TCP playback-control API. The PulseAudio bridge uses its own private Unix socket, output-only modules and no TCP audio service.

These controls exclude other users and browser TCP callers from the control socket. They do not defend against malicious software already running as the same user, or an exploited/replaced engine with host privileges. The proprietary Soloist implementation cannot receive a complete source audit here.

## Sources

- [Official Soloist announcement and lossless support](https://developer.spotify.com/blog/2026-08-13-introducing-spotify-soloist)
- [Official Soloist documentation](https://developer.spotify.com/documentation/soloist)
- [Official WebSocket protocol](https://developer.spotify.com/documentation/soloist/reference/websocket-api)
- [Mac compatibility runtime](https://github.com/RyanTheTechMan/soloist)
- [Upstream Spotifast](https://github.com/crmne/spotifast)

The [complete runtime patch and build instructions](../runtime/README.md) accompany this fork. Startup, private API, simulated protocol tests and authenticated audio are reported separately.
