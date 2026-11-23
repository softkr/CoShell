# CoShell's copy of winit 0.30.13

This is winit 0.30.13 from crates.io with two changes, in `src/platform_impl/macos/view.rs`
(search for "CoShell patch").

**What it fixes.** Right after switching to Apple's Korean input method, macOS answers the first
key with a bare `insertText:` carrying one jamo instead of starting a composition. winit forwards
that as an ordinary key, so "한" reaches the app as "ㅎ", "ㅏ", "ㄴ". The patch notices exactly that
answer (no marked text before or after, one `insertText:` with a single Hangul compatibility jamo,
Apple Korean input source, not a key repeat) and hands the same key event to the input method once
more; the second time it composes.

**Second fix: dropping the composition when IME is turned off.** `set_ime_allowed(false)` clears
winit's own marked text but never tells the system, so the input method keeps the syllable and
inserts it with the next key, into whichever field has focus by then ("콘솔" in one field, then
"솔ㅇ" in the next). The patch calls `discardMarkedText` on the input context there, as
`unmarkText` already does. CoShell's IME wrapper turns the input method off for one frame after it
commits a syllable itself, which is what triggers this.

**Upstream.** Same approach as rust-windowing/winit#4744 (and #4759 for the 0.30 branch), neither
merged when this copy was made. When a winit release includes the fix, delete this directory and
the `[patch.crates-io]` entry in the workspace `Cargo.toml`.
