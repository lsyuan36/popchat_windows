# PopChat app icon

`app.ico` is the original two-blue-chat-bubble artwork from
`src/ui/tray.rs`, rendered at 16, 24, 32, 48, 64, 128 and 256 pixels.
The geometry, blue colors, straight alpha and 4×4 coverage sampling are
shared with the runtime tray/window renderer in `src/app_icon.rs`.
The original 32px tray pixels are preserved exactly.

This is project-authored artwork under the repository's MIT license;
there are no fonts, downloaded graphics, AI-generated images or new dependencies.
The ICO contains uncompressed 32-bit BGRA DIBs with transparency masks.

To regenerate after intentionally changing the shared renderer, compile a
temporary Rust program that imports `src/app_icon.rs` using an absolute
`#[path = r"..."] mod app_icon;` and calls:

```rust
std::fs::write("assets/app-icon/app.ico", app_icon::ico_bytes()).unwrap();
```

`build.rs` verifies the checked-in ICO is current, then writes header-free
resource scripts and compiled `.res` files **only inside Cargo's OUT_DIR**.
The Windows SDK `rc.exe` is discovered from PATH or installed SDK locations.
It links resources only to `popchat_windows` and `popchat_updater`; package
version metadata follows Cargo automatically. Non-Windows targets skip PE
resources. Windows resource compilation currently requires an MSVC target.
