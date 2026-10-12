---
layout: home
title: Spotifast
description: A fast, lightweight Spotify app for Linux, macOS, and Windows.
permalink: /
hero:
  name: Spotifast
  text: Spotify, native and fast
  tagline: A lightweight Spotify app for Linux, macOS, and Windows. Listen on your computer, browse your music, and control your other devices.
  actions:
    - theme: brand
      text: Download
      link: /download/
    - theme: alt
      text: What is Spotifast?
      link: /what-is-spotifast/
    - theme: alt
      text: GitHub
      link: https://github.com/crmne/spotifast
  image:
    src: /screenshot.png
    alt: "Spotifast Home with the playlist library, recommendations, queue, and player visible"
    width: 2018
    height: 1198

features:
  - icon: ⚡
    title: Lightweight
    details: Opens in well under a second and typically uses just 100–250 MB of memory.
  - icon: 🔊
    title: Spotify Connect
    details: Listen on your computer with smooth transitions between songs, or control music on a speaker, phone, or TV. Playback needs Spotify Premium.
  - icon: 📚
    title: Library and search
    details: Browse playlists, Liked Songs, albums, artists, and podcasts. Search the catalogue and edit playlists you own.
  - icon: 🎵
    title: Focused music interface
    details: One dark appearance with album artwork, library, search, playlists and playback controls.
  - icon: ⌨️
    title: Desktop controls
    details: Use keyboard shortcuts and your keyboard's media keys. Keep the music playing after you close the window.
  - icon: 🔓
    title: Open source
    details: Free to use, study, and improve. Explore the code, report a problem, or help build the next version.
    link: https://github.com/crmne/spotifast
    link_text: Read the source
---

<video class="hero-film image-src" controls muted loop playsinline preload="metadata" poster="/assets/images/launch-film-poster.jpg" aria-label="Spotifast in a minute: a film of the app, made with its built-in demo content" hidden>
  <source src="/assets/videos/launch-film.mp4" type="video/mp4">
</video>

<script>
  // The theme's hero takes a picture; the film takes its place where
  // scripts run, and the screenshot stays where they do not.
  (function () {
    var film = document.querySelector(".hero-film");
    var slot = document.querySelector(".VPHero .image-container");
    if (!film || !slot) return;
    slot.replaceChildren(film);
    film.hidden = false;
    if (!window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      film.play().catch(function () {});
    }
  })();
</script>

<style>
  .hero-film {
    display: block;
    background: #0b0e0c;
  }
</style>

<style>
  /* The hero image slot is sized for a square logo; the screenshot needs the
     room. Page-scoped overrides, so the theme stays untouched. */
  .VPHero .image-container {
    width: 100% !important;
    height: auto !important;
    transform: none !important;
  }
  .VPHero .image-src {
    position: relative !important;
    top: auto !important;
    left: auto !important;
    transform: none !important;
    width: 100% !important;
    height: auto !important;
    max-width: 100% !important;
    max-height: none !important;
    padding: 0 !important;
    border-radius: 12px;
    box-shadow: 0 12px 48px rgba(0, 0, 0, 0.45);
  }
  @media (max-width: 959px) {
    .VPHero .image {
      margin: 0 0 24px !important;
    }
  }
</style>
