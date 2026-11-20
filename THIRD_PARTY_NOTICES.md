# Third-party notices

Flimmer ships the following third-party software and data. Each part keeps its own license.

## FFmpeg (ffmpeg.exe, ffprobe.exe)

- Build: 9.0.2 "essentials" build for Windows by Gyan Doshi, https://www.gyan.dev/ffmpeg/builds/
- License: GNU General Public License v3.0 (this build enables GPL components). The full text is in
  `tools/ffmpeg/LICENSE` next to the executables.
- Source code: FFmpeg itself is at https://github.com/FFmpeg/FFmpeg (tag `n9.0.2`), https://ffmpeg.org/download.html.
  The build scripts and the sources of the bundled libraries are listed on https://www.gyan.dev/ffmpeg/builds/.
- Flimmer runs these programs as separate processes; they are not linked into Flimmer.

## uBlock Origin Lite (desktop/extensions/ubol)

- Version 2026.1116.1650, by Raymond Hill and contributors.
- License: GNU General Public License v3.0, full text in `desktop/extensions/ubol/LICENSE.txt`.
- Source code: https://github.com/uBlockOrigin/uBOL-home
- Loaded unmodified into the hidden browser windows that look for streams.

## Libraries compiled into Flimmer.exe

- librqbit (torrent engine), Apache License 2.0, https://github.com/ikatson/rqbit
- BoringTun (userspace WireGuard), BSD 3-Clause License, https://github.com/cloudflare/boringtun
- smoltcp (TCP/IP stack), 0BSD, https://github.com/smoltcp-rs/smoltcp
- Tauri, MIT or Apache License 2.0, https://github.com/tauri-apps/tauri
- axum, tokio, reqwest, rusqlite (with SQLite, public domain), serde and other Rust crates under MIT, Apache 2.0 or
  BSD licenses. The full list with versions is in `Cargo.lock` of the source repository.

## Web interface

- hls.js, Apache License 2.0, https://github.com/video-dev/hls.js
- React, React Router, TanStack Query, MIT License
- Bricolage Grotesque and Instrument Sans fonts (via Fontsource), SIL Open Font License 1.1

## Torrent search

The torrent search sources follow the approach of tork by melqtx, MIT License, https://github.com/melqtx/tork

## Movie data

Titles, images and descriptions come from TMDB, https://www.themoviedb.org/.
This product uses the TMDB API but is not endorsed or certified by TMDB.
Each user needs their own free TMDB API Read Access Token; none is included.
