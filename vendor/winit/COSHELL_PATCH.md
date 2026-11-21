# CoShell's copy of winit 0.30.13

This is winit 0.30.13 from crates.io with one change, in `src/platform_impl/macos/view.rs`
(search for "CoShell patch").

**What it fixes.** Right after switching to Apple's Korean input method, macOS answers the first
key with a bare `insertText:` carrying one jamo instead of starting a composition. winit forwards
that as an ordinary key, so "한" reaches the app as "ㅎ", "ㅏ", "ㄴ". The patch notices exactly that
answer (no marked text before or after, one `insertText:` with a single Hangul compatibility jamo,
Apple Korean input source, not a key repeat) and hands the same key event to the input method once
more; the second time it composes.

**Upstream.** Same approach as rust-windowing/winit#4744 (and #4759 for the 0.30 branch), neither
merged when this copy was made. When a winit release includes the fix, delete this directory and
the `[patch.crates-io]` entry in the workspace `Cargo.toml`.
