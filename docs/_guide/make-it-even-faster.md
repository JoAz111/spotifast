---
title: Requests and Performance
description: How this fork limits Spotify traffic and keeps playback local.
nav_order: 6
---

This fork uses your local Spotify Developer Client ID. It does not fall back to
the original shared registration. Keep the ID in `.private/spotify-client-id`
before building, and register `http://127.0.0.1:8989/login` in Spotify's dashboard.
No Client Secret is needed. Changing the ID requires Spotify sign-in again.

Spotify still applies quotas to your registration. The client honors the complete
`Retry-After` interval, rechecks cooldowns for queued requests, and paces request
starts 500 ms apart. While Soloist is active, playback state comes from its local
control API instead of periodic Web API polling. Playlist metadata can use the
Spotify metadata session where supported.

A development registration has endpoint restrictions and ten-result search pages.
A private registration cannot create access to removed or unavailable endpoints.
See [How It Connects](/how-it-connects/) for the actual paths and limitations.

The window draws as needed and follows platform frame pacing. The interface has
one dark appearance; skins, custom themes, visualizers and EQ have been removed.
