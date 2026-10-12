# Cloud performance audit

Baseline: `83c273e388bcbe1c56202dd36c8a2fe9b41dec77`, verified locally before editing.
Candidate branch: `codex/cloud-performance-audit`. Review target: `codex/soloist-macos`.
No interface layout, audio-engine, runtime patch, dependency, theme or authentication
registration changes are included.

## Implemented

- Web API admission now selects foreground page/search/play work before optional
  Home and enrichment work. Each priority class stays FIFO. Only the queue head
  owns the pacing/cooldown timer; other waiters sleep on change notifications.
  Waiting work owns no active request capacity. Active requests remain limited to
  six and starts remain spaced by 500 ms per authorization.
- Foreground library, full Top Songs and Recents-panel requests bypass the optional
  background semaphore, so foreground intent reaches admission even when four
  optional jobs are waiting on Spotify.
- Token preparation precedes admission. A synchronous expiry and grant-revocation
  check follows admission, preventing slow token refresh from gathering admitted
  requests into a burst and preventing a prepared token from surviving revocation.
  Expiry during a long cooldown causes preparation again before sending.
- Existing ordered queue-write locking and retry semantics remain intact.
  Foreground priority cannot preempt an HTTP request that has already started;
  continuous foreground traffic can delay optional shelves.
- Sidebar folder counts use a bulk prefix-count traversal instead of rescanning
  the rootlist for each folder. Nested, duplicate, unloaded and incomplete
  rootlists retain their counting semantics. The first duplicate folder id wins.
  Existing painting, pins, filtering, drag insertion and playback behavior remain.
- Removed an orphaned comment about the deleted author website that caused strict
  Clippy to attach it to an unrelated settings-width constant.

## Measured fixture

`python3 tools/profile-folder-counts.py` extracts the actual old helper from the
baseline and the actual new helper from current source. The isolated Rust fixture
uses the same enum field layout and counts all sibling folders in a synthetic
rootlist. Nine sequential samples per size, baseline first then candidate, median
reported; allocation and id lookup are included. Both sums are checked.

| Compiler mode | Folders / rootlist rows | Baseline median | Candidate median |
| --- | --- | --- | --- |
| Debug, rustc opt-level=0 | 512 / 1,536 | 2.039727 ms | 0.348895 ms |
| Debug, rustc opt-level=0 | 4,096 / 12,288 | 127.459789 ms | 2.893391 ms |
| Optimized, rustc opt-level=3 | 512 / 1,536 | 0.493933 ms | 0.029164 ms |
| Optimized, rustc opt-level=3 | 4,096 / 12,288 | 33.321380 ms | 0.401714 ms |

Rust 1.98.0, Linux x86_64 cloud environment. The optimized fixture is **not a
release-profile application benchmark**. These results measure folder counting,
not full frames, total CPU/RAM, startup, Spotify latency, energy or audio. No
percentage improvement is claimed for the application.

The synthetic paused-time admission fixture queues two background and then two
foreground requests during a 120-second cooldown. Its expected order is
foreground 3, foreground 4, background 1, background 2 at the paced starts.
Cancellation, capacity, cooldown extension and task-local scope have separate tests.
A FIFO-only probe failed the foreground-order and capacity-priority assertions;
the production gate passed all five isolated tests with Tokio 1.53.1.

## Linux verification scope

Before the final review follow-ups to the background cap and token preparation,
the cloud checkout produced:

| Check | Observed result |
| --- | --- |
| Formatting over all tracked Rust sources plus admission module | Passed using rustfmt directly |
| Default/all-feature all-target strict lint | Both passed using clippy-driver with `-D warnings` |
| Default `cargo test --locked --all-targets` | Library: 871 passed, 9 failed, 1 ignored |
| All-feature `cargo test --locked --all-targets --all-features` | Library: 894 passed, 9 failed, 1 ignored |
| All-feature doctests | Passed, 0 doctests |
| Warning-strict all-feature Rustdoc, no deps | Passed |
| Launcher packaging regressions | 4 passed |
| Flatpak metainfo regressions | 4 passed |
| Release-name regressions | 4 passed |
| Jekyll site build | Not qualified; missing gems initially, workspace later filled during dependency setup |
| Nix/native package installation | Not run; no Nix environment or native package qualification here |

Standard Cargo fmt and Clippy wrappers require `/proc/self/exe`, unavailable in
this cloud sandbox. Workspace-local Rust and development libraries, an explicit
sysroot, GNU linker and one build job made the underlying checks feasible.
No project profiles, lints, lockfiles or tests were weakened to accommodate this.
Rust test compilation succeeded; the library failures stopped Cargo before the
other test targets ran, so the numbers above are not all-target pass counts.

The final review follow-ups add token-readiness, prepared-token expiry and
revocation-during-admission regressions, and strengthen the foreground classification
test to check the background cap. They were authored through the GitHub connector
after the filesystem filled and the command sandbox could no longer start.
**Those final changes require fresh CI/local checks; the preceding passes must not
be attributed to the final commit.** The PR records any subsequent CI results.

All four corrected checkpoint fixtures passed on Linux: custom titlebar recreation,
library drag autoscroll, long Playing Next autoscroll, and queue/player insertion
at the chosen playlist position. Real ordering, insertion and no-playback-change
assertions were preserved. The checkpoint's eight performance regressions remain.

### Nine native failures in both library runs

Three fail at `std::env::current_exe()` because `/proc/self/exe` is absent:

- `app::tests::mpris_search_links_open_search_on_a_private_bus`
- `backend::authorization_tests::proxy_changes_compare_with_the_running_engine`
- `zeroconf::tests::receiver_probes_bypass_an_environment_proxy`

The remaining six hit denied Unix socket/IPC operations:

- `single_instance::tests::a_client_reaches_the_command_queue_and_the_snapshot`
- `single_instance::tests::a_second_launch_hands_its_link_to_the_first`
- `soloist::unix::tests::authentication_loss_cancels_activation_load_without_replaying_it`
- `soloist::unix::tests::load_keeps_admitted_account_token_and_context_offsets_after_activation`
- `soloist::unix::tests::priority_pause_precedes_saturated_mutations_and_cancels_active_load`
- `soloist::unix::tests::socket_requires_private_directory_owned_socket_and_no_symlinks`

Four Soloist tests fail while binding the fixture socket with `Operation not permitted`;
the single-instance command round trip also returns that error, and second launch
cannot surface the first instance. These failures are unqualified, not skipped or
counted as passing. A successful exact-baseline full-suite run was not obtained in
this sandbox, so no claim of an all-green baseline is made.

## Remaining performance work

- Sidebar entry construction still clones, filters and sorts the full selected
  shelf every repaint. A derived-row cache needs explicit revisions for playlist
  pagination, blank-header fill, optimistic and confirmed rename, artwork updates,
  add/remove and ordering; plus account, grants, rootlist, collapsed folders,
  pins, history ordering, filter, sort, locale and album/podcast classifications.
  Collection length and pointer identity cannot prove freshness. This audit did
  not introduce a partial cache with unreliable invalidation.
- Logical API activity spans rate-limit waits, and visible spinners request frames
  every 33 ms. Reducing expensive frames during long cooldowns remains pending.
  The transport gate is event-driven; this does not itself eliminate UI animation.
- Account-bound metadata snapshots for immediately useful stale content remain
  pending, including one refresh generation and fresh permission checks before edits.
- Full application CPU/RAM, startup/reload and release-profile measurements need a
  real native host and representative library/visible-page fixtures.

## Mac qualification and privacy

The existing local chat retains Mac builds, native tests, packaging, startup and
live playback. Re-run the exact final PR commit, all-feature targets and full tests,
then verify sign-in failure without a local Client ID, receiver startup, device
strip, drag insertion/order, page/search responsiveness during cooldown and
pause/resume/song changes. The official Soloist executable and runtime patch were
not modified. No real Spotify account was accessed in cloud tests.

No keys, private Client ID, grants, session state, user caches, raw engine logs,
Soloist executable or personal application archives are in this change. Token
fixtures use synthetic registrations and dummy credentials; new request fixtures
are restricted to loopback and cancel before sending or reject revoked admission.
Previously demonstrated 44.1 kHz FLAC and smooth audio with a 500 ms Mac buffer
do not certify bit-perfect output or the closed engine.
