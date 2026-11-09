// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use commentcleanup::{app, check, session, update};

use std::path::PathBuf;

fn flag(args: &[String], name: &str) -> Option<String> {
    let index = args.iter().position(|a| a == name)?;
    args.get(index + 1).cloned()
}

fn absolute(path: String) -> PathBuf {
    std::path::absolute(&path).unwrap_or_else(|_| path.into())
}

/// What the taskbar and Alt+Tab show while the app is running
fn window_icon() -> Option<iced::window::Icon> {
    let bytes: &[u8] = include_bytes!("../assets/icon-48.png");
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().ok()?;
    let mut rgba = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut rgba).ok()?;
    iced::window::icon::from_rgba(rgba, frame.width, frame.height).ok()
}

fn main() -> iced::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|first| first == "--check") {
        std::process::exit(check::run(&args[1..]));
    }
    if update::available() {
        update::clean_up();
    }
    let options = app::Options {
        folder: args.first().filter(|a| !a.starts_with("--")).cloned().map(absolute),
        open: flag(&args, "--open").map(absolute),
        theme: flag(&args, "--theme"),
        standalone: args.iter().any(|a| a == "--standalone"),
        autoscroll: args.iter().any(|a| a == "--autoscroll"),
        step: flag(&args, "--step").and_then(|s| s.parse().ok()).unwrap_or(400.0),
        shot: flag(&args, "--shot").map(Into::into),
        delay: flag(&args, "--delay").and_then(|s| s.parse().ok()).unwrap_or(3000),
        exit: args.iter().any(|a| a == "--exit"),
    };

    iced::application(move || app::App::boot(options.clone()), app::App::update, app::App::view)
        .title(app::App::title)
        .subscription(app::App::subscription)
        .theme(app::App::theme)
        .default_font(iced::Font::with_name("Segoe UI"))
        .window(iced::window::Settings {
            size: iced::Size::new(1280.0, 800.0),
            min_size: Some(iced::Size::new(640.0, 400.0)),
            visible: session::load().window.is_none(),
            exit_on_close_request: false,
            decorations: false,
            icon: window_icon(),
            platform_specific: iced::window::settings::PlatformSpecific {
                undecorated_shadow: true,
                ..Default::default()
            },
            ..Default::default()
        })
        .run()
}
