---
title: How It Connects
description: Private Spotify authorization and local Soloist playback.
nav_order: 5
---

This fork uses the user's Spotify Developer registration for the Web API.
`build.rs` reads `.private/spotify-client-id`, a Git-ignored local file, and embeds
its value in the local executable. No shared upstream registration or Client
Secret is shipped in tracked source. Missing configuration fails visibly at
sign-in. Authorization uses PKCE, state validation and the loopback callback
`http://127.0.0.1:8989/login`. A saved Web API grant is restored only if its Client
ID matches this build, so changing registrations requires signing in again.

The internal credential slot still named `Shared` is the primary slot for this
local registration. Its name does not imply a shared developer quota. Legacy
personal-slot records remain readable for migration; the old personal-app setup
and upsell are no longer in the interface.

## Music and playback

The Web API supplies account/library/search data and supported playback/context
operations. A metadata-only librespot session supplies Spotify playlist/folder,
lyrics and other available music metadata. Its Spotify desktop playback identity
is distinct from the Web API registration. In Soloist mode it does not create a
second audio player or Connect receiver.

The official, unmodified Soloist executable owns playback. An experimental Apple
Silicon compatibility runtime uses Hypervisor.framework for Linux ARM64 code and
host syscall translation; it does not boot a guest Linux operating system. The
native launcher owns its lifecycle and opens the music window after the private
control socket is ready. The key is saved in Keychain and reused on later launches.
Initial Spotify sign-in and Connect pairing still require the user's account.

The client connects to an owner-only Unix socket (0700 directory, 0600 socket),
checks ownership/type/peer identity, bounds messages and commands, and rejects
stale connection/authentication generations. No TCP WebSocket listener is exposed.
The launcher passes the receiver identity from its owned private state file to the
frontend. That exact identity marks Soloist as this computer and deduplicates its
Connect row; names alone do not establish a local device.
Spotify Connect discovery and initial pairing still use the LAN and can trigger
macOS firewall consent for elfuse. Same-user processes and root remain trusted.

Playback commands wait for actual receiver activation before loading Web API
contexts. Pause has priority over queued commands. The app uses Soloist's local
state/events and avoids periodic Web API playback polling while Soloist is active;
explicit refreshes remain available. Queue operations preserve their ordering and
do not replay after a connection changes.

## Rate limits

Since [23 July 2026](https://developer.spotify.com/blog/2026-07-23-web-api-quota-updates),
Spotify counts development-mode quota per developer account across all its Client
IDs. Using this user's registration removes the upstream developer's shared quota
dependency; it does not remove Spotify's own limits. A 429 can indicate either rate
limiting or an exhausted quota (`QUOTA_EXCEEDED`).

API requests are paced at most once per 500 ms per authorization. Every queued
request rechecks the shared cooldown after acquiring an in-flight slot. A Spotify
429 response preserves the full `Retry-After` period; it is not capped at 30 seconds.
Requests resume at the paced rate, avoiding a release burst. Ordered queue writes
keep their existing order while waiting. This reduces unnecessary traffic, but
cannot remove Spotify's quota or development-mode endpoint restrictions.

Search and artist-album page sizes use the development-mode limit of 10. Some
features require endpoints Spotify does not grant to development apps. Metadata
session reads can supply some of those data; unsupported requests must surface
unavailability, never restore a third-party app registration as a fallback.

## Other traffic and credentials

See [Privacy](/privacy/) for outbound destinations. Lyrics come only from Spotify;
artwork is restricted to Spotify CDN hosts. Optional GitHub updates target the
user's fork and default off. Proxy settings apply to the shared HTTP client and
metadata connection. Local receiver control never uses an external proxy.

Keychain grants are account-bound and generation-checked before persistence.
Private local records and pairing caches must not be included in source, logs or
packages. A public OAuth Client ID can be extracted from the local executable;
keeping it out of Git is a source-distribution preference, not a secret boundary.

This Mac has decoded FLAC at 44.1 kHz and passed user tests for song changes,
pause/resume and smooth playback with a 500 ms buffer. That evidence does not
establish source bit depth, bit-perfect hardware output, full Spotify parity or
support on other operating systems.
