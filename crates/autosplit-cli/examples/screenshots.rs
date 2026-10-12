//! Renders the windows to PNG files without a display, to look at the layout:
//! `cargo run --example screenshots -- <folder>`.

use std::rc::Rc;

use autosplit::gui::messages::Buttons;
use autosplit::gui::ui::{
    AboutWindow, HotkeyRow, InfoCell, InstructionCard, InstructionsWindow, LogWindow, MainWindow, MakerRow,
    MakerWindow, MessageWindow, SettingsWindow, SimilarityRow, TesterWindow,
};
use autosplit::gui::{HOTKEY_ROWS, file_name_parts, transparency_grid};
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
    main.set_tool("Live Tester".into());
    save(&window, PhysicalSize::new(786, 516), &folder.join("main-tester-open.png"));
    main.set_tool("".into());
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

    let tester = TesterWindow::new().unwrap();
    tester.set_has_image(true);
    let file = "05_Bottle_(0.93)_[35]_#1100#_@0@_^0^_{b}.png";
    tester.set_file(file.into());
    tester.set_file_parts(file_name_parts(file));
    let cell =
        |name: &str, value: &str, note: &str| InfoCell { name: name.into(), value: value.into(), note: note.into() };
    let info = vec![
        cell("ACTS", "splits once it drops back below", ""),
        cell("METHOD", "L2 Norm", "from profile"),
        cell("THRESHOLD", "0.930", ""),
        cell("DELAY", "1100 ms", ""),
    ];
    tester.set_info(std::rc::Rc::new(slint::VecModel::from(info)).into());
    tester.set_ignored("Ignored in the name: the loop count 0 isn't 1 or more, so the image is used once".into());
    tester.set_status("Reached the threshold. Waiting for it to drop back below.".into());
    tester.set_can_reload(true);
    tester.set_can_next_image(true);
    tester.set_live_message("Waiting for capture device...".into());
    tester.set_split_image(solid([60, 90, 200]));
    tester.set_kept_image(solid([200, 60, 60]));
    tester.set_kept_caption("Reached: 0.951".into());
    tester.set_kept_position("1/2".into());
    tester.set_can_next_kept(true);
    tester.set_live("0.951".into());
    tester.set_highest("0.962".into());
    tester.set_threshold("0.930".into());
    tester.set_reached(true);
    tester.set_live_value(0.951);
    tester.set_highest_value(0.962);
    tester.set_highest_reached(true);
    tester.set_threshold_value(0.93);
    tester.show().unwrap();
    save(&window, PhysicalSize::new(992, 470), &folder.join("tester.png"));
    tester.set_reached(false);
    tester.set_live("0.412".into());
    tester.set_live_value(0.412);
    tester.set_highest("0.962".into());
    tester.set_highest_value(0.962);
    tester.set_highest_reached(true);
    save(&window, PhysicalSize::new(992, 470), &folder.join("tester-waiting.png"));
    tester.hide().unwrap();

    let maker = MakerWindow::new().unwrap();
    let presets = ["The Legend of Zelda: Ocarina of Time: No Wrong Warp".into()];
    maker.set_presets(std::rc::Rc::new(slint::VecModel::from(presets.to_vec())).into());
    let row = |file: &str, source: &str, saved: bool| MakerRow {
        parts: file_name_parts(file),
        file: file.into(),
        source: source.into(),
        saved,
    };
    let rows = vec![
        row("reset_(0.96)_[0.1].png", "000_SplitImage.png", true),
        row("start_auto_splitter_(0.96)_[10]_{b}.png", "001_SplitImage.png", true),
        row("02_DekuDummy_(0.94)_[6.5]_{d}.png", "002_SplitImage.png", false),
        row("03_EnterDeku_(0.92)_[60]_#1000#.png", "", false),
    ];
    maker.set_rows(std::rc::Rc::new(slint::VecModel::from(rows)).into());
    maker.set_selected(2);
    maker.set_output_folder("C:/Splits/OoT".into());
    // Half the split image is transparent, over the transparency grid
    let half =
        (0..320 * 240).flat_map(|i| if i % 320 < 160 { [60, 90, 200, 255] } else { [0, 0, 0, 0] }).collect::<Vec<u8>>();
    let half = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&half, 320, 240);
    maker.set_preview_image(slint::Image::from_rgba8(half));
    maker.set_grid_image(transparency_grid(320, 240));
    maker.set_preview_message("".into());
    maker.set_preview_caption("DekuDummy: split image".into());
    maker.set_status("Saved 2 split images, moved 2 screenshots to the base folder.".into());
    maker.set_can_save_selected(true);
    maker.set_can_save_all(true);
    maker.set_can_test(true);
    maker.set_has_savestates(true);
    maker.set_has_instructions(true);
    maker.show().unwrap();
    maker.set_live_message("".into());
    maker.set_live_image(solid([90, 140, 90]));
    maker.set_can_grab(true);
    save(&window, PhysicalSize::new(830, 610), &folder.join("maker.png"));
    maker.hide().unwrap();

    let instructions = InstructionsWindow::new().unwrap();
    let card = |heading: &str, savestate: &str, text: &str| InstructionCard {
        heading: heading.into(),
        savestate: savestate.into(),
        text: text.into(),
    };
    let cards = vec![card(
        "03. EnterDeku",
        "savestates/006 enter deku.gzs",
        "After loading, advance 1 frame before taking your screenshot. The A button doesn't render correctly if the savestate is on the frame itself, so it had to be 1 frame before.",
    )];
    instructions.set_cards(std::rc::Rc::new(slint::VecModel::from(cards)).into());
    instructions.show().unwrap();
    save(&window, PhysicalSize::new(520, 200), &folder.join("instructions.png"));
    instructions.hide().unwrap();

    let log = LogWindow::new().unwrap();
    log.set_log("    0.001s  Loaded settings.toml\n    0.002s  Timer: hotkeys (split num 1)".into());
    log.show().unwrap();
    save(&window, PhysicalSize::new(640, 360), &folder.join("log.png"));
}
