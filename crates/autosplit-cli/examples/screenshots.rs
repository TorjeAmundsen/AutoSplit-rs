//! Renders the windows to PNG files without a display, to look at the layout:
//! `cargo run --example screenshots -- <folder>`.

use std::rc::Rc;

use autosplit::gui::messages::Buttons;
use autosplit::gui::preset_form::{FADE_MASK_NAME_CHARS, MASK_NAME_CHARS, SAVESTATE_NAME_CHARS, elide_middle};
use autosplit::gui::ui::{
    AboutWindow, EntryStep, HotkeyRow, InfoCell, InlineMessage, InstructionCard, InstructionsWindow,
    LibraryEntryWindow, LibraryItem, LibraryStep, LibraryWindow, LogWindow, MainWindow, MakerRow, MakerWindow,
    MessageWindow, SettingsWindow, SimilarityRow, SplitForm, SplitValues, TesterReading, TesterWindow,
};
use autosplit::gui::{HOTKEY_ROWS, file_name_parts, name_parts_model, thumbnail, transparency_grid};
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

/// A Split Library file name written with `XX` for its number, in the parts the window colors,
/// the number's like a number's.
fn library_parts(name: &str) -> slint::ModelRc<autosplit::gui::ui::NamePart> {
    let mut parts = autosplit_core::filename::name_parts(&name.replacen("XX", "0", 1));
    if let Some(number) = parts.first_mut().filter(|part| part.kind == autosplit_core::filename::PartKind::Number) {
        number.text = "XX".to_owned();
    }
    name_parts_model(parts)
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
    main.set_window_title("AutoSplit v0.3.0".into());
    main.show().unwrap();
    save(&window, PhysicalSize::new(786, 450), &folder.join("main.png"));
    // A message box drawn in the window, like under Wayland
    let question = InlineMessage {
        shown: true,
        title: "AutoSplit".into(),
        message: "Do you want to save changes made to settings file settings.toml?".into(),
        primary: "Yes".into(),
        secondary: "No".into(),
        tertiary: "Cancel".into(),
        default_button: 0,
    };
    main.set_inline_message(question.clone());
    save(&window, PhysicalSize::new(786, 450), &folder.join("main-inline-message.png"));
    main.set_inline_message(InlineMessage::default());
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
    main.invoke_screenshot_saved("003_SplitImage.png".into());
    main.set_tool("Live Tester".into());
    main.set_running(true);
    main.set_can_reset(true);
    main.set_can_skip(true);
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
        save(&window, PhysicalSize::new(380, 414), &folder.join(format!("settings{tab}.png")));
    }
    settings.hide().unwrap();

    let about = AboutWindow::new().unwrap();
    about.set_version("0.3.0".into());
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
    // A long list scrolls, with buttons that say what they do
    let files: Vec<String> = (0..40).map(|index| format!("{index:02}_Split_(0.95).png")).collect();
    message.set_message(format!("These split images are replaced:\n{}", files.join("\n")).into());
    message.set_primary("Delete Old Images".into());
    message.set_secondary("Keep Old Images".into());
    message.set_default_button(1);
    save(&window, PhysicalSize::new(380, 500), &folder.join("message-long.png"));
    message.hide().unwrap();

    let tester = TesterWindow::new().unwrap();
    tester.set_has_image(true);
    let file = "05_Bottle_(0.93)_[35]_#1100#_@0@_^0^_{b}.png";
    tester.set_file(file.into());
    tester.set_file_parts(file_name_parts(file));
    let cell = |name: &str, value: &str, note: &str| InfoCell {
        name: name.into(),
        value: value.into(),
        note: note.into(),
        wide: name == "ACTS",
    };
    let info = vec![
        cell("ACTS", "splits once it drops back below", ""),
        cell("METHOD", "L2 Norm", "from profile"),
        cell("THRESHOLD", "0.930", ""),
        cell("DELAY", "1100 ms", ""),
    ];
    tester.set_info(std::rc::Rc::new(slint::VecModel::from(info)).into());

    tester.set_can_reload(true);
    tester.set_can_next_image(true);

    tester.set_split_image(solid([60, 90, 200]));
    tester.set_kept_image(solid([200, 60, 60]));
    tester.set_kept_caption("Reached: 0.951".into());
    tester.set_kept_position("1/2".into());
    tester.set_can_next_kept(true);
    let reached = TesterReading {
        ignored: "Ignored in the name: the loop count 0 isn't 1 or more, so the image is used once".into(),
        status: "Reached the threshold. Waiting for it to drop back below.".into(),
        live_message: "Waiting for capture device...".into(),
        live: "0.951".into(),
        highest: "0.962".into(),
        threshold: "0.930".into(),
        reached: true,
        highest_reached: true,
        live_value: 0.951,
        highest_value: 0.962,
        threshold_value: 0.93,
    };
    tester.set_reading(reached.clone());
    tester.show().unwrap();
    save(&window, PhysicalSize::new(992, 470), &folder.join("tester.png"));
    tester.set_reading(TesterReading { reached: false, live: "0.412".into(), live_value: 0.412, ..reached });
    save(&window, PhysicalSize::new(992, 470), &folder.join("tester-waiting.png"));
    tester.hide().unwrap();

    let maker = MakerWindow::new().unwrap();
    let games = ["The Legend of Zelda: Ocarina of Time".into()];
    maker.set_games(std::rc::Rc::new(slint::VecModel::from(games.to_vec())).into());
    let presets = ["No Wrong Warp".into()];
    maker.set_presets(std::rc::Rc::new(slint::VecModel::from(presets.to_vec())).into());
    // A grab, half blue and half green
    let shot = autosplit_core::image::BgraImage::from_raw(
        64,
        48,
        (0..64 * 48).flat_map(|i| if i % 64 < 32 { [200, 120, 60, 255] } else { [60, 160, 60, 255] }).collect(),
    );
    let thumb = slint::Image::from_rgba8(thumbnail(shot.view()));
    let row = |heading: &str, file: &str, source: &str, saved: bool| MakerRow {
        heading: heading.into(),
        parts: file_name_parts(file),
        file: file.into(),
        source: source.into(),
        saved,
        thumb: if source.is_empty() { slint::Image::default() } else { thumb.clone() },
        has_thumb: !source.is_empty(),
        problem: false,
        joined: false,
        shaded: false,
    };
    let rows = vec![
        row("Reset Image", "reset_(0.96)_[0.1].png", "000_SplitImage.png", true),
        row("Start Image", "start_auto_splitter_(0.96)_[10]_{b}.png", "grabbed", true),
        row("DekuDummy", "02_DekuDummy_(0.94)_[6.5]_{d}.png", "002_SplitImage.png", false),
        row("EnterDeku", "03_EnterDeku_(0.92)_[60]_#1000#.png", "", false),
    ];
    maker.set_rows(std::rc::Rc::new(slint::VecModel::from(rows)).into());
    maker.set_selected(2);
    maker.set_output_folder("G:/AutoSplit segmented practice/VCAM/Portable OBS/AutoSplit/OoT".into());
    // Half the split image is transparent, over the transparency grid
    let half =
        (0..320 * 240).flat_map(|i| if i % 320 < 160 { [60, 90, 200, 255] } else { [0, 0, 0, 0] }).collect::<Vec<u8>>();
    let half = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&half, 320, 240);
    maker.set_preview_image(slint::Image::from_rgba8(half));
    maker.set_grid_image(transparency_grid(320, 240));
    maker.set_preview_message("".into());
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
    // Without advanced, the folder is above the list and the window two rows shorter
    save(&window, PhysicalSize::new(708, 612), &folder.join("maker-simple.png"));
    maker.set_advanced(true);
    save(&window, PhysicalSize::new(828, 672), &folder.join("maker.png"));
    // A fade split selected, which isn't made from the capture
    maker.set_grab_fade(true);
    save(&window, PhysicalSize::new(828, 672), &folder.join("maker-fade.png"));
    maker.set_grab_fade(false);
    // A preset written by hand that has to be fixed before anything's made from it
    maker.set_preset_problem(
        "Boss Alt: It has the same name as a split above it that it's compared together with. Give it another name."
            .into(),
    );
    maker.set_can_grab(false);
    // And no split image folder yet
    let output_folder = maker.get_output_folder();
    maker.set_output_folder("".into());
    save(&window, PhysicalSize::new(828, 672), &folder.join("maker-problem.png"));
    maker.set_output_folder(output_folder);
    maker.set_preset_problem("".into());
    maker.set_can_grab(true);
    // Just after a grab: the split above saved, the selected one next to grab
    maker.invoke_split_images_saved(slint::ModelRc::from([false, true].as_slice()), false);
    // Past the check mark popping in
    std::thread::sleep(std::time::Duration::from_millis(300));
    slint::platform::update_timers_and_animations();
    save(&window, PhysicalSize::new(828, 672), &folder.join("maker-saved.png"));
    // A long message drawn in the window scrolls
    let files: Vec<String> = (1..=40).map(|number| format!("{number:02}_Boss_(0.95).png")).collect();
    maker.set_inline_message(InlineMessage {
        message: format!(
            "These split images are replaced:
{}

Replace them?",
            files.join(
                "
"
            )
        )
        .into(),
        primary: "Replace".into(),
        secondary: "".into(),
        tertiary: "Cancel".into(),
        ..question
    });
    save(&window, PhysicalSize::new(828, 672), &folder.join("maker-inline-message.png"));
    maker.set_inline_message(InlineMessage::default());
    // What was done still shows beside the grouping warning
    maker.set_needs_grouping(true);
    maker.set_status("Copied 3 savestates to the clipboard.".into());
    save(&window, PhysicalSize::new(828, 672), &folder.join("maker-grouping.png"));
    maker.set_needs_grouping(false);
    // Enough splits to scroll, which puts the scroll bar over the rows' right side
    let many: Vec<MakerRow> = (0..14)
        .map(|index| {
            row("Boss", &format!("{index:02}_Boss_(0.95).png"), if index % 2 == 0 { "grabbed" } else { "" }, true)
        })
        .collect();
    maker.set_rows(std::rc::Rc::new(slint::VecModel::from(many)).into());
    save(&window, PhysicalSize::new(828, 672), &folder.join("maker-long.png"));
    // Editing, with the masked capture below the form and a split that needs a mask
    let mut rows = vec![
        row("Reset Image", "reset_(0.96)_[0.1].png", "000_SplitImage.png", false),
        row("Start Image", "start_auto_splitter_(0.96)_[10]_{b}.png", "grabbed", false),
        row("DekuDummy", "02_DekuDummy_(0.94)_[6.5]_{d}.png", "002_SplitImage.png", false),
        row("EnterDeku", "03_EnterDeku_(0.92)_[60]_#1000#.png", "", false),
        row("EnterDekuBackflip", "03_EnterDekuBackflip_(0.9)_[60]_#1000#.png", "", false),
        row("Split1", "04_Split1_(0.95).png", "", false),
    ];
    rows[4].joined = true;
    for (index, shaded) in [false, true, false, true, true, false].into_iter().enumerate() {
        rows[index].shaded = shaded;
    }
    rows[5].problem = true;
    maker.set_rows(std::rc::Rc::new(slint::VecModel::from(rows)).into());
    maker.set_selected(3);
    maker.set_editing(true);
    let kinds: Vec<slint::SharedString> = ["Split", "Start Image", "Reset Image"].map(Into::into).into();
    maker.set_kinds(std::rc::Rc::new(slint::VecModel::from(kinds)).into());
    let games: Vec<slint::SharedString> =
        ["The Legend of Zelda: Ocarina of Time", "New game..."].map(Into::into).into();
    maker.set_edit_games(std::rc::Rc::new(slint::VecModel::from(games)).into());
    maker.set_edit_game_index(0);
    maker.set_edit_game("The Legend of Zelda: Ocarina of Time".into());
    maker.set_edit_name("No Wrong Warp".into());
    maker.set_has_split(true);
    let values = maker.global::<SplitValues>();
    values.set_name("EnterDeku".into());
    values.set_has_threshold(true);
    values.set_threshold(0.92);
    values.set_has_delay(true);
    values.set_delay(1000.0);
    values.set_has_pause(true);
    values.set_pause(60.0);
    values.set_instructions("After loading, advance 1 frame before taking your screenshot.".into());
    maker.set_form(SplitForm {
        named: true,
        name_problem: "".into(),
        mask_problem: "".into(),
        color_problem: "".into(),
        drops_savestate: false,
        group_problem: "".into(),
        fade_color: Default::default(),
        mask: elide_middle("006_a_button_flip_after_the_door.png", MASK_NAME_CHARS).into(),
        savestate: elide_middle("006 enter deku.gzs", SAVESTATE_NAME_CHARS).into(),
        mask_path: "presets/No_Wrong_Warp/006_a_button_flip.png".into(),
        savestate_path: "C:/Users/Runner/gz/006 enter deku.gzs".into(),
        mask_copied: false,
        savestate_copied: true,
        grouping_off: false,
        can_join: true,
        why_not_joined: "".into(),
    });
    maker.set_problems("Split1: It needs a mask. Choose one.".into());
    maker.set_has_problems(true);
    maker.set_saved_preset(true);
    save(&window, PhysicalSize::new(828, 672), &folder.join("maker-edit.png"));
    // Where a ready-made split from Split Library would go
    maker.set_drop_line(3);
    save(&window, PhysicalSize::new(828, 672), &folder.join("maker-edit-drop.png"));
    maker.set_drop_line(-1);
    // Without the savestate note, all of the form fits
    maker.set_form(SplitForm { savestate_copied: false, ..maker.get_form() });
    save(&window, PhysicalSize::new(828, 672), &folder.join("maker-edit-plain.png"));
    // A fade split of the whole screen
    values.set_compare(1);
    values.set_color("#a0a0a0".into());
    values.set_threshold(0.995);
    maker.set_form(SplitForm {
        mask: "".into(),
        fade_color: slint::Color::from_rgb_u8(0xa0, 0xa0, 0xa0),
        // Its savestate from before, which saving leaves out
        drops_savestate: true,
        ..maker.get_form()
    });
    save(&window, PhysicalSize::new(828, 672), &folder.join("maker-edit-fade.png"));
    // A fade split of part of the screen, whose mask can be cleared
    maker.set_form(SplitForm {
        mask: elide_middle("deku_without_the_title.png", FADE_MASK_NAME_CHARS).into(),
        ..maker.get_form()
    });
    save(&window, PhysicalSize::new(828, 672), &folder.join("maker-edit-fade-mask.png"));
    values.set_compare(0);
    values.set_threshold(0.92);
    maker.set_form(SplitForm {
        mask: elide_middle("006_a_button_flip_after_the_door.png", MASK_NAME_CHARS).into(),
        drops_savestate: false,
        ..maker.get_form()
    });
    // Compared with the split above while the profile has grouping off
    values.set_with_previous(true);
    maker.set_form(SplitForm { grouping_off: true, ..maker.get_form() });
    save(&window, PhysicalSize::new(828, 672), &folder.join("maker-edit-grouping.png"));
    values.set_with_previous(false);
    maker.set_form(SplitForm { grouping_off: false, ..maker.get_form() });
    maker.set_form(SplitForm { savestate_copied: true, ..maker.get_form() });
    // A new split's mask and savestate, both copied in on save
    maker.set_form(SplitForm { mask_copied: true, ..maker.get_form() });
    save(&window, PhysicalSize::new(828, 672), &folder.join("maker-edit-copied.png"));
    maker.set_form(SplitForm { mask_copied: false, ..maker.get_form() });
    // A long problem wraps under the name
    maker.set_form(SplitForm {
        name_problem:
            "It has the same name as a split above it that it's compared together with. Give it another name.".into(),
        group_problem: "There's no split before it to compare it with.".into(),
        mask_problem: "It needs a mask. Choose one.".into(),
        ..maker.get_form()
    });
    save(&window, PhysicalSize::new(828, 672), &folder.join("maker-edit-problem.png"));
    maker.hide().unwrap();

    let instructions = InstructionsWindow::new().unwrap();
    let card = |heading: &str, savestate: &str, text: &str| InstructionCard {
        split: 3,
        heading: heading.into(),
        savestate: savestate.into(),
        text: text.into(),
    };
    let cards = vec![card(
        "EnterDeku",
        "03_EnterDeku_SEE_INSTRUCTIONS.gzs",
        "After loading, advance 1 frame before taking your screenshot. The A button doesn't render correctly if the savestate is on the frame itself, so it had to be 1 frame before.",
    )];
    instructions.set_cards(std::rc::Rc::new(slint::VecModel::from(cards)).into());
    instructions.show().unwrap();
    save(&window, PhysicalSize::new(520, 200), &folder.join("instructions.png"));
    instructions.hide().unwrap();

    // The bundled library's base images, and a boss that takes two images, then one that's in
    // the preset already and one that can't be added
    let library = LibraryWindow::new().unwrap();
    let base = |file: &str| {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../splits/OcarinaOfTime").join(file);
        slint::Image::load_from_path(&path).unwrap()
    };
    let step =
        |heading: &str, file: &str, [threshold, delay, pause]: [&str; 3], dummy: bool, below: bool, name: &str| {
            LibraryStep {
                file_parts: library_parts(name),
                file: name.into(),
                heading: heading.into(),
                thumb: base(file),
                threshold: threshold.into(),
                delay: delay.into(),
                pause: pause.into(),
                dummy,
                below,
                fade: Default::default(),
                // As release builds show it
                savestate: "".into(),
                savestate_shown: false,
                savestate_missing: false,
            }
        };
    let library_item =
        |heading: &str, description: &str, steps: Vec<LibraryStep>, in_preset: bool, blocker: &str| LibraryItem {
            in_preset_note: if in_preset { "Already in the preset, as split 05.".into() } else { "".into() },
            ready_made: 0,
            heading: heading.into(),
            description: description.into(),
            steps: std::rc::Rc::new(slint::VecModel::from(steps)).into(),
            blocker: blocker.into(),
        };
    let items = vec![
        library_item(
            "Ganon",
            "Final slash on Ganon: the intro text, then the flash",
            vec![
                step("Ganon intro", "base_collapse_cs.png", ["", "", ""], true, false, "XX_Ganon intro_{d}.png"),
                step("Ganon", "base_ganon.png", ["0.92", "", ""], false, false, "XX_Ganon_(0.92).png"),
            ],
            false,
            "",
        ),
        library_item(
            "Bottle",
            "Splits on bottle model over head, common manual split timing",
            vec![step(
                "Bottle",
                "base_bottle.png",
                ["0.93", "1100 ms", "35 s"],
                false,
                true,
                "XX_Bottle_(0.93)_[35]_#1100#_{b}.png",
            )],
            true,
            "",
        ),
        library_item(
            "Reset Image",
            "Automatically reset when pressing C-up in Link's house",
            vec![step("Reset Image", "base_reset.png", ["0.96", "", "0.1 s"], false, false, "reset_(0.96)_[0.1].png")],
            false,
            "A preset can have up to 4 reset images, and this would make 5.",
        ),
    ];
    library.set_items(std::rc::Rc::new(slint::VecModel::from(items)).into());
    library.set_window_title("Split Library - The Legend of Zelda: Ocarina of Time".into());
    library.set_errors("A split library couldn't be read.".into());
    library.set_selected(0);
    library.show().unwrap();
    save(&window, PhysicalSize::new(680, 520), &folder.join("library.png"));
    library.set_selected(1);
    // Developer builds can change and delete what's in the library
    library.set_developer(true);
    save(&window, PhysicalSize::new(680, 420), &folder.join("library-single.png"));
    save(&window, PhysicalSize::new(480, 300), &folder.join("library-narrow.png"));
    library.set_items(std::rc::Rc::new(slint::VecModel::from(Vec::<LibraryItem>::new())).into());
    library.set_errors("".into());
    library.set_search_text("ganon bottle".into());
    library.set_empty_message("No ready-made splits match ganon bottle.".into());
    save(&window, PhysicalSize::new(600, 300), &folder.join("library-empty.png"));
    library.hide().unwrap();

    // A boss of two images made from the preset's selected splits
    let entry = LibraryEntryWindow::new().unwrap();
    let entry_step = |heading: &str, file: &str, name: &str, source: &str| EntryStep {
        heading: heading.into(),
        name: heading.into(),
        renamable: true,
        thumb: base(file),
        file_parts: library_parts(name),
        file: name.into(),
        source: source.into(),
        picked: source.starts_with("Base image from base_"),
    };
    let steps = vec![
        entry_step(
            "Ganon intro",
            "base_collapse_cs.png",
            "XX_Ganon intro_{d}.png",
            "Base image from base_ganon_intro.png",
        ),
        entry_step("Ganon", "base_ganon.png", "XX_Ganon_(0.92).png", "Base image from the live capture"),
    ];
    entry.set_game("The Legend of Zelda: Ocarina of Time".into());
    entry.set_description_text("Final slash on Ganon: the intro text, then the flash".into());
    entry.set_images(2.0);
    entry.set_most_images(4.0);
    entry.set_steps(std::rc::Rc::new(slint::VecModel::from(steps)).into());
    entry.show().unwrap();
    save(&window, PhysicalSize::new(520, 300), &folder.join("library-entry.png"));
    entry.set_problem(
        "CrawlspaceRight has no grab or screenshot, and there's no live capture, to make its base image from.".into(),
    );
    entry.set_steps(std::rc::Rc::new(slint::VecModel::from(Vec::<EntryStep>::new())).into());
    entry.set_editing(true);
    entry.set_change(2);
    save(&window, PhysicalSize::new(520, 240), &folder.join("library-entry-problem.png"));
    entry.hide().unwrap();

    let log = LogWindow::new().unwrap();
    log.set_log("    0.001s  Loaded settings.toml\n    0.002s  Timer: hotkeys (split num 1)".into());
    log.show().unwrap();
    save(&window, PhysicalSize::new(640, 360), &folder.join("log.png"));
}
