# CoShell's copy of iced_wgpu 0.14.0

This is iced_wgpu 0.14.0 from crates.io with one change, in `src/window/compositor.rs`
(search for "CoShell patch").

**What it is for.** iced configures the window's surface as a render target only, so nothing can
read back what has been drawn in a frame. CoShell's AI sheet is frosted glass, like macOS's: its
shader (`crates/coshell-app/src/glow.rs`) copies the part of the frame behind the sheet, blurs it
and draws it back under the conversation. The patch adds `COPY_SRC` to the surface's usage when the
system supports it (Metal, Vulkan and DirectX do); where it does not, the sheet draws a plain tint.

**Upstream.** There is no option for this in iced. If a later iced lets the surface's usage be
chosen, or offers a backdrop blur, delete this directory and its `[patch.crates-io]` entry in the
workspace `Cargo.toml`.
