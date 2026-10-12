# Soloist on macOS

This fork uses the unchanged official Linux ARM64 Soloist engine on Apple Silicon through the community elfuse macOS compatibility runtime. It is an experimental personal test build. Spotify does not publish a native macOS Soloist engine. The compatibility runtime is not a security sandbox and has the invoking user's host file and network authority.

The native Rust interface remains Spotifast. With `SPOTIFAST_SOLOIST_SOCKET` set by the launcher, all local audio comes from Soloist. The optional librespot session supplies playlist folders, playlist metadata, lyrics and radio; it creates no player, audio output or second Spotify Connect receiver. Web API sign-in remains separate.

## Music features

The main interface includes Home, songs, artists, albums, playlists, Liked Songs, queue, lyrics and podcasts. The Library's Podcasts shelf opens followed shows, Search includes podcast and episode results, and Home shows episodes to continue or new episodes from followed podcasts. Episode playback keeps its saved resume position. Legacy `music_only` settings are ignored so earlier previews cannot keep these features hidden.

Local controls support play/pause, next/previous, seek, volume, ordinary shuffle, repeat, queue additions and Connect activation. Album/playlist playback retains the selected row, full context, filtered track list, duplicates and position by activating Soloist first, then using Spotify's Web API. Explicit shuffle and repeat choices are sent in sequence with that load. A failed or disconnected mutation is never automatically replayed.

Soloist's local protocol exposes neither the account ID nor the Connect device ID.
The launcher reads the actual device ID from the receiver's owned private state
file, so its Connect destination is treated as this Mac and appears once in the
device picker. The green destination strip is reserved for another device. Pair Soloist and sign into Spotifast with the **same Spotify account**. A private socket verifies the local OS process, not account equality. A device switch in another Spotify client can race a Web API load targeting the active device; this API limitation is not hidden by guessing a device name.

## Current limitations

- Premium and a personal Soloist API key are required. Keys are entered locally in a native secure field and stored in Keychain, never in chat or source.
- Official Soloist supports Premium lossless up to 24-bit/44.1 kHz. This Mac has authenticated, decoded FLAC at 44.1 kHz and passed user tests for track changes, pause/resume and smooth sound with a 500 ms buffer. A successful startup or float32 audio sink alone does not prove lossless or bit-perfect output.
- Queue clear, removal and reordering are disabled for Soloist because its documented local API does not provide them. Queue queries request at most 80 items. Unsolicited queue broadcasts can contain only 10; query and broadcast messages have no IDs, so a racing broadcast can be a current but shortened snapshot.
- Crossfade/Automix and other engine capabilities do not automatically become interface controls. Smart Shuffle, Jam, DJ, offline downloads and full Spotify feature parity are not implemented here.
- Skins, custom themes, Winamp layouts, equalizer and visualizers are removed. There is one dark appearance. Soloist audio output selection lives under Settings → Audio output….
- The compatibility runtime targets Apple Silicon. Intel Mac, Windows and Linux Soloist integration have not been qualified by this change.

## Current-track quality

Soloist's documented local state has no codec or quality field. The metadata
session therefore has a read-only candidate observer for Spotify's Connect
cluster feed. It registers no extra Connect device and never starts a player.
A metadata-only session may receive no cluster messages; live delivery has not
been qualified. Missing data leaves the badge absent.

The Lossless label requires actual HiFi/HiFi24 quality with HiFi status on,
the owned receiver identity, the current track, a matching local generation,
and evidence younger than 15 seconds. Target quality and availability flags
cannot enable it. Track changes, buffering, disconnection, stale data and
unknown/lossy quality clear it. An unanchored repeat of the same URI remains
unknown, because Soloist provides no playback-occurrence identifier. Decoder
log history and output sample format are not used to guess a current-track label.

## Local setup and lifecycle

The combined native launcher selects the official engine file stored separately from the app. The official binary is not bundled or committed. Use Spotify's documented download endpoint and renew expired builds according to its Soloist documentation.

The packaged runtime uses a dedicated `~/Library/Application Support/Spotifast Soloist` profile. The Rust app also uses its own `spotifast-soloist` profile so it does not migrate or overwrite the original Spotifast profile. The key and each Spotify grant are separate credentials.

Paste the Soloist key once and continue. Later launches start the saved receiver and open the music window automatically. Initial Connect pairing and Spotify browser sign-in still require the same account. The Web API uses a locally embedded developer Client ID from the Git-ignored `.private/spotify-client-id` file; it never falls back to the upstream shared registration. Optional **Playlist and lyrics access** in Settings opens the separate streaming-permission browser approval needed for internal metadata reads. Normal library and catalog access use the Web API grant.

Quitting the music interface stops its owned receiver. Spotifast sign-out cancels pending commands and requests an explicit pause; it does not revoke Soloist's independently paired session. The launcher's reset-pairing action clears only its initialized local receiver state and cache, retaining the API key. It does not revoke a cloud account grant.

## Control boundary

The raw Soloist WebSocket API has no client authentication. This runtime remaps only its selected guest control listener to a private Unix socket: parent directory mode 0700, socket 0600. The Rust client checks the owner, permissions, peer UID and socket identity, bounds messages and command admission, and discards old account/connection operations. There is no TCP fallback for that API.

Spotify Connect discovery and pairing are separate network services. macOS can therefore ask whether `elfuse` may accept incoming connections. That prompt is not evidence of a TCP playback-control API. The PulseAudio bridge uses its own private Unix socket, output-only modules and no TCP audio service.

These controls exclude other users and browser TCP callers from the control socket. They do not defend against malicious software already running as the same user, or an exploited/replaced engine with host privileges. The proprietary Soloist implementation cannot receive a complete source audit here.

## Control responsiveness

Active pause, resume, seek and volume use the private local socket and bypass Web
API pacing. Play/pause commands carry an explicit desired state, so repeated
clicks do not toggle a stale receiver snapshot. Delayed opposite state echoes do
not erase a newer UI intent. Commands and watchdogs progress while state frames
arrive continuously. Volume dragging keeps one command in flight and one latest
pending value; release preserves the final value without queueing every slider
step. Selecting music still requires a Spotify Web API load, but unchanged repeat
and shuffle options do not add requests before it.

Runtime health queries and automatic output routing run in one bounded owned
subprocess while the supervisor continues draining engine output. A fresh healthy
API response remains valid if only routing stalls; routing delays do not restart
healthy audio. The subprocess is stopped and reaped on timeout and shutdown.
These changes have protocol and lifecycle regression coverage. They require a
rebuilt runtime helper; a frontend-only package refresh cannot include them.
Fixture results and command acknowledgements do not prove audible smoothness.

## Sources

- [Official Soloist announcement and lossless support](https://developer.spotify.com/blog/2026-08-13-introducing-spotify-soloist)
- [Official Soloist documentation](https://developer.spotify.com/documentation/soloist)
- [Official WebSocket protocol](https://developer.spotify.com/documentation/soloist/reference/websocket-api)
- [Mac compatibility runtime](https://github.com/RyanTheTechMan/soloist)
- [Upstream Spotifast](https://github.com/crmne/spotifast)

The [complete runtime patch and build instructions](../runtime/README.md) accompany this fork. Startup, private API, simulated protocol tests and authenticated audio are reported separately.

## Home listening mix

Spotify's [November 2024 Web API change](https://developer.spotify.com/blog/2024-11-27-changes-to-the-web-api)
removes Recommendations access from new/development registrations. This fork does
not send that unsupported request or fall back to somebody else's registration.
Home instead shows **Inspired by your listening**, a Spotify song-radio preview
seeded by one of your top songs. This is distinct from Spotify's personalized Home
feed and its former multi-seed Web API Recommendations endpoint.

The shelf uses the existing metadata session. Its inline **Enable music access**
button opens the separate Spotify authorization when needed. A visible shelf
resolves one station and fetches metadata for at most 20 unique songs, with a
20-second deadline and one outstanding request. Hiding it prevents requests.
Missing listening history and failed reads settle visibly. Retry reloads just this
mix, or just the top-song seeds if history failed, while existing songs stay on
screen. Replaced requests are canceled; stale account/session or retry results
cannot replace the current mix. No third-party music or recommendation service is
used.
