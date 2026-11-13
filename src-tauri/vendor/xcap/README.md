# XCap

English | [简体中文](README-zh_CN.md)

XCap is a cross-platform screen capture library written in Rust. It supports Linux (X11, Wayland), MacOS, and Windows. XCap supports screenshot and video recording (to be implemented).

## Features

-   Cross-platform: Supports Linux (X11, Wayland), MacOS, and Windows.
-   Supports multiple screenshot modes: Can take screenshots of the screen and windows.
-   Supports video recording: Supports recording of the screen or window (to be implemented).

### Implementation Status

| Feature          | Linux(X11) | Linux(Wayland) | MacOS | Windows |
| ---------------- | ---------- | -------------- | ----- | ------- |
| Screen Capture   | ✅         | ⛔             | ✅    | ✅      |
| Window Capture   | ✅         | ⛔             | ✅    | ✅      |
| Screen Recording | 🛠️         | 🛠️             | 🛠️    | 🛠️      |
| Window Recording | 🛠️         | 🛠️             | 🛠️    | 🛠️      |

-   ✅: Feature available
-   ⛔: Feature available, but not fully supported in some special scenarios
-   🛠️: To be developed

## Examples

-   Screen Capture

```rust
use std::time::Instant;
use xcap::Monitor;

fn normalized(filename: &str) -> String {
    filename
        .replace("|", "")
        .replace("\\", "")
        .replace(":", "")
        .replace("/", "")
}

fn main() {
    let start = Instant::now();
    let monitors = Monitor::all().unwrap();

    for monitor in monitors {
        let image = monitor.capture_image().unwrap();

        image
            .save(format!("target/monitor-{}.png", normalized(monitor.name())))
            .unwrap();
    }

    println!("运行耗时: {:?}", start.elapsed());
}
```

-   Window Capture

```rust
use std::time::Instant;
use xcap::Window;

fn normalized(filename: &str) -> String {
    filename
        .replace("|", "")
        .replace("\\", "")
        .replace(":", "")
        .replace("/", "")
}

fn main() {
    let start = Instant::now();
    let windows = Window::all().unwrap();

    let mut i = 0;

    for window in windows {
        // 最小化的窗口不能截屏
        if window.is_minimized() {
            continue;
        }

        println!(
            "Window: {:?} {:?} {:?}",
            window.title(),
            (window.x(), window.y(), window.width(), window.height()),
            (window.is_minimized(), window.is_maximized())
        );

        let image = window.capture_image().unwrap();
        image
            .save(format!(
                "target/window-{}-{}.png",
                i,
                normalized(window.title())
            ))
            .unwrap();

        i += 1;
    }

    println!("运行耗时: {:?}", start.elapsed());
}
```

## Linux System Requirements

On Linux, you need to install `libxcb`, `libxrandr`, and `dbus`.

Debian/Ubuntu:

```sh
apt-get install libxcb1 libxrandr2 libdbus-1-3
```

Alpine:

```sh
apk add libxcb libxrandr dbus
```

ArchLinux:

```sh
pacman -S libxcb libxrandr dbus
```

## License

This project is licensed under the Apache License. See the [LICENSE](./LICENSE) file for details.
