# App icon

`openmango.icon` is the supplied Mango Icon Composer project, including its light,
dark, and tinted appearances. Keep the project and its three layer images together.

Run `just app-icon` on macOS with Xcode 26 or newer to compile it. The generated
`target/app-icon/Assets.car` and `openmango.icns` provide the current macOS icon and
the fallback for older macOS releases. Both are included by the release script.
The asset compiler also supplies the bundle icon keys in `icon-info.plist`.

After changing the source artwork, copy `target/app-icon/openmango.png` to
`assets/logo/openmango.png`. This 256-pixel export serves the 120-point welcome
screen icon and 128-pixel README image at Retina resolution. Rust builds embed only
this export; they do not require Xcode to compile the icon project.

The original download's posters, wordmark, font family, and duplicate loose layers
are not app runtime assets and are not included here.

See [Apple's Icon Composer documentation](https://developer.apple.com/documentation/xcode/creating-your-app-icon-using-icon-composer)
for appearance and backward-compatibility behavior.
