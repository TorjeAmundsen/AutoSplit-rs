//! Renders the windows to PNG files without a display, to look at the layout:
//! `cargo run --example screenshots -- <folder>`.

use std::rc::Rc;

use autosplit::gui::HOTKEY_ROWS;
use autosplit::gui::messages::Buttons;
use autosplit::gui::ui::{AboutWindow, HotkeyRow, LogWindow, MainWindow, MessageWindow, SettingsWindow, SimilarityRow};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType, Rgb565Pixel};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, PhysicalSize};

struct Headless {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
}

fn save(window: &MinimalSoftwareWindow, size: PhysicalSize, path: &std::path::Path) {
    window.set_size(size);
    let mut pixels = vec![Rgb565Pixel::default(); (size.width * size.height) as usize];
    window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, size.width as usize);
    });
    let rgb: Vec<u8> = pixels
        .iter()
        .flat_map(|pixel| {
            let value = pixel.0;
            let (r, g, b) = ((value >> 11) & 0x1f, (value >> 5) & 0x3f, value & 0x1f);
            [(r << 3 | r >> 2) as u8, (g << 2 | g >> 4) as u8, (b << 3 | b >> 2) as u8]
        })
        .collect();
    image::save_buffer(path, &rgb, size.width, size.height, image::ExtendedColorType::Rgb8).unwrap();
}

fn main() {
    let folder = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless { window: window.clone() })).unwrap();

    let main = MainWindow::new().unwrap();
    main.set_device_name("OBS Virtual Camera".into());
    main.set_live_message("Waiting for capture device...".into());
    main.set_current_image_file("05_Bottle_(0.93)_[35]_#1100#_{b}.png".into());
    main.set_current_image_message("None (Paused). 9.9 seconds remaining".into());
    main.set_window_title("AutoSplit v0.1.0".into());
    main.show().unwrap();
    save(&window, PhysicalSize::new(786, 450), &folder.join("main.png"));
    let row = |live: &str, highest: &str, threshold: &str| SimilarityRow {
        live: live.into(),
        highest: highest.into(),
        threshold: threshold.into(),
    };
    let rows =
        vec![row("0.412", "0.951", "0.930"), row("0.200", "0.800", "0.900"), row("-", "-", "-"), row("-", "-", "-")];
    main.set_group_rows(std::rc::Rc::new(slint::VecModel::from(rows)).into());
    main.set_group_split_images(true);
    save(&window, PhysicalSize::new(786, 516), &folder.join("main-grouped.png"));
    let solid = |rgb: [u8; 3]| {
        let pixels = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
            &[rgb[0], rgb[1], rgb[2], 255].repeat(4),
            2,
            2,
        );
        slint::Image::from_rgba8(pixels)
    };
    let tiles = vec![solid([200, 60, 60]), solid([60, 160, 60]), solid([60, 90, 200]), slint::Image::default()];
    main.set_current_tiles(std::rc::Rc::new(slint::VecModel::from(tiles)).into());
    main.set_current_grid(true);
    main.set_current_image_file("Comparing against 3 images".into());
    main.set_current_image_message("".into());
    save(&window, PhysicalSize::new(786, 516), &folder.join("main-grid.png"));
    main.hide().unwrap();

    let settings = SettingsWindow::new().unwrap();
    let hotkeys: Vec<HotkeyRow> = HOTKEY_ROWS
        .iter()
        .map(|&(_, name)| HotkeyRow { name: name.into(), hotkey: "num 1".into(), enabled: true })
        .collect();
    settings.set_hotkeys(std::rc::Rc::new(slint::VecModel::from(hotkeys)).into());
    settings.set_setting_hotkey(2);
    settings.show().unwrap();
    for tab in 0..4 {
        settings.invoke_show_tab(tab);
        save(&window, PhysicalSize::new(380, 384), &folder.join(format!("settings{tab}.png")));
    }
    settings.hide().unwrap();

    let about = AboutWindow::new().unwrap();
    about.set_version("0.1.0".into());
    about.show().unwrap();
    save(&window, PhysicalSize::new(264, 250), &folder.join("about.png"));
    about.hide().unwrap();

    let message = MessageWindow::new().unwrap();
    message.set_message("Do you want to save changes made to settings file settings.toml?".into());
    let [primary, secondary, tertiary] = Buttons::YesNoCancel.labels();
    message.set_primary(primary.into());
    message.set_secondary(secondary.into());
    message.set_tertiary(tertiary.into());
    message.show().unwrap();
    save(&window, PhysicalSize::new(380, 120), &folder.join("message.png"));
    message.hide().unwrap();

    let log = LogWindow::new().unwrap();
    log.set_log("    0.001s  Loaded settings.toml\n    0.002s  Timer: hotkeys (split num 1)".into());
    log.show().unwrap();
    save(&window, PhysicalSize::new(640, 360), &folder.join("log.png"));
}
