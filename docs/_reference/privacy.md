---
title: Privacy
description: Local storage and outbound connections in this Soloist fork.
nav_order: 4
---

This fork runs locally. Source review found no analytics SDK, telemetry uploader,
crash-report upload, advertising service, or hosted Spotifast backend. Logs stay
on the Mac. This is a source-review conclusion, not proof about every packet or
the internals of Spotify's closed-source Soloist executable.

## Local storage

Spotify authorization grants and the Soloist key use macOS Keychain. Spotify
passwords are entered on Spotify's website. Settings, library/history snapshots,
artwork and Spotify lyrics are local. The receiver has a separate private
Application Support profile for its installation, pairing state and audio cache.
Never share that profile. Sign out removes the client authorization; resetting
receiver pairing is a separate operation.

The developer Client ID is loaded at build time from the ignored local file
`.private/spotify-client-id`. It is embedded in the local executable, not tracked
source. A Client Secret is neither required nor accepted. A build without a local
ID cannot silently fall back to the upstream registration.

## Outbound connections

| Destination | Trigger and information |
| --- | --- |
| Spotify accounts/API/services | Sign-in, search, library, playlists, music metadata, lyrics and playback control under your Spotify account. |
| Spotify artwork CDNs (`scdn.co`, `spotifycdn.com` and subdomains) | Visible/prefetched artwork. Other artwork hosts are refused. The shared asynchronous HTTP client does not follow redirects. |
| Local Spotify Connect receivers | mDNS discovery and explicit pairing/control. The owned Soloist control socket is local and owner-only. |
| GitHub (`api.github.com`, `github.com`, release asset hosts) | Optional update checks/downloads from `JoAz111/spotifast`, the user's fork. Automatic checks/downloads default off; existing preferences can be disabled in Settings. No listening metadata is attached by the updater. |
| Configured proxy | If enabled, requests follow your proxy preference. The proxy operator can observe traffic appropriate to that proxy. |

The LRCLIB fallback was removed. Opening lyrics no longer sends song artist,
title, album or duration to a third-party lyrics service. Skin/preset downloaders
were removed along with Winamp and MilkDrop. Fonts and icons are bundled.

Links deliberately opened in your browser are separate from background app
traffic. Spotify controls its own service and CDN infrastructure and may use
infrastructure providers; this source review is not a server-ownership guarantee.

Cargo, package managers and build scripts contact dependency registries and
source/download hosts during development. Those are not music-client telemetry.
