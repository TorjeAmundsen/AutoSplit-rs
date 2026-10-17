//! The preset editor's draft: edits, list operations kept in step with other lists, and problems.

use autosplit::gui::preset_form::{
    Draft, Field, InstructionText, ListChange, Problem, SplitEdit, SplitKind, discard_question, elide_middle,
    field_problems, field_value, import_blocker, in_preset, instruction_cards, known_game, library_file_name,
    library_heading, matches_search, next_problem_split, problem_flags, problem_list, problem_summary,
    saved_preset_name, split_heading, status_line,
};
use autosplit_core::preset::{Preset, PresetSplit};
use test_case::test_case;

fn split(name: &str) -> PresetSplit {
    PresetSplit { name: name.into(), mask: format!("{name}.png"), ..PresetSplit::default() }
}

fn draft(names: &[&str]) -> Draft {
    Draft::edit(Preset {
        name: "Any%".into(),
        game: "Game".into(),
        splits: names.iter().map(|name| split(name)).collect(),
    })
}

fn names(draft: &Draft) -> Vec<&str> {
    draft.preset.splits.iter().map(|split| split.name.as_str()).collect()
}

#[test_case("Any%", &[], true => Some("Any%".to_owned()); "a free name")]
#[test_case("Any%", &["any%"], true => Some("Any% (2)".to_owned()); "a copy")]
#[test_case("Any% ", &["Any%", "Any% (2)"], true => Some("Any% (3)".to_owned()); "the next free copy")]
#[test_case("Any%  NMS", &["any% nms"], false => None; "saved in place under a taken name")]
#[test_case("Any%", &["100%"], false => Some("Any%".to_owned()); "saved in place")]
fn presets_are_saved_under_names_their_game_tells_apart(name: &str, taken: &[&str], new: bool) -> Option<String> {
    // Act, with the expected value checked by test_case
    saved_preset_name(name, taken, new)
}

#[test]
fn instruction_cards_are_named_like_the_copied_savestates() {
    // Arrange: two reset images and splits compared together share numbers, and Door has no
    // instructions, so list places would be off
    let mut draft = grouped(&["reset", "reset", "A", "+B", "Door", "C"]);
    for (index, instructions) in [(1, "Wait"), (3, "Jump"), (4, ""), (5, "Run")] {
        draft.edit_split(index, SplitEdit::Savestate("savestates/state.gzs".into()));
        draft.edit_split(index, SplitEdit::Instructions(instructions.into()));
    }

    // Act
    let cards = instruction_cards(&draft.preset);

    // Assert
    let card = |split: usize, heading: &str, file: &str, instructions: &str| InstructionText {
        split,
        heading: heading.into(),
        file: file.into(),
        instructions: instructions.into(),
    };
    assert_eq!(
        cards,
        [
            card(1, "Reset Image 2", "0_reset_2_SEE_INSTRUCTIONS.gzs", "Wait"),
            card(3, "B", "1_B_SEE_INSTRUCTIONS.gzs", "Jump"),
            card(5, "C", "3_C_SEE_INSTRUCTIONS.gzs", "Run"),
        ]
    );
}

#[test]
fn an_unchanged_preset_has_no_changes() {
    // Act
    let draft = draft(&["Boss"]);

    // Assert
    assert!(!draft.has_changes());
}

#[test]
fn editing_back_to_how_it_was_has_no_changes() {
    // Arrange
    let mut draft = draft(&["Boss"]);
    draft.edit_split(0, SplitEdit::Name("Door".into()));

    // Act
    draft.edit_split(0, SplitEdit::Name("Boss".into()));

    // Assert
    assert!(!draft.has_changes());
}

#[test]
fn a_new_preset_can_be_saved() {
    // Act
    let draft = Draft::new("Game");

    // Assert
    assert_eq!((draft.can_save(), names(&draft), draft.preset.game.as_str()), (true, vec!["Split1"], "Game"));
}

#[test_case(None => None; "the profile's")]
#[test_case(Some(0.95) => Some(0.95); "its own, also like the profile's")]
#[test_case(Some(0.9954) => Some(0.995); "rounded like the spin box")]
#[test_case(Some(1.5) => Some(1.0); "at most 1")]
fn thresholds_are_the_splits_own_or_the_profiles(threshold: Option<f64>) -> Option<f64> {
    // Arrange
    let mut draft = draft(&["Boss"]);

    // Act
    draft.edit_split(0, SplitEdit::Threshold(threshold));

    // Assert, with the expected value checked by test_case
    draft.preset.splits[0].threshold
}

#[test]
fn instructions_wait_for_a_savestate_until_saving() {
    // Arrange
    let mut draft = draft(&["Boss"]);
    draft.edit_split(0, SplitEdit::Savestate("boss.gzs".into()));
    draft.edit_split(0, SplitEdit::Instructions("Advance a frame.".into()));

    // Act
    draft.edit_split(0, SplitEdit::Savestate(String::new()));

    // Assert: kept for another savestate, but not saved without one
    let kept = draft.preset.splits[0].instructions.as_str();
    assert_eq!((kept, draft.to_save().splits[0].instructions.as_str()), ("Advance a frame.", ""));
}

#[test]
fn added_splits_go_after_the_selected_one_with_a_free_name() {
    // Arrange
    let mut draft = draft(&["Split1", "Boss"]);

    // Act
    let added = draft.add(Some(0));

    // Assert
    assert_eq!((added, names(&draft)), ((1, ListChange::Insert(1)), vec!["Split1", "Split2", "Boss"]));
}

#[test]
fn without_a_selection_splits_are_added_at_the_end() {
    // Arrange
    let mut draft = draft(&["Boss"]);

    // Act
    let added = draft.add(None);

    // Assert
    assert_eq!(added, (1, ListChange::Insert(1)));
}

#[test]
fn duplicates_go_after_their_split() {
    // Arrange
    let mut draft = draft(&["Boss", "Door"]);

    // Act
    let duplicated = draft.duplicate(0);

    // Assert
    assert_eq!((duplicated, names(&draft)), (Some((1, ListChange::Insert(1))), vec!["Boss", "Boss", "Door"]));
}

#[test_case(&["Boss", "Door"], 1 => Some((0, ListChange::Remove(1))); "last selects the one before")]
#[test_case(&["Boss", "Door"], 0 => Some((0, ListChange::Remove(0))); "first selects the next")]
#[test_case(&["Boss"], 0 => Some((0, ListChange::Remove(0))); "only split selects where the next would be")]
#[test_case(&["Boss"], 3 => None; "no such split")]
fn removing_selects_a_neighbor(splits: &[&str], index: usize) -> Option<(usize, ListChange)> {
    // Act, with the expected value checked by test_case
    draft(splits).remove(index)
}

#[test_case(1, true => Some((0, ListChange::Rotate { start: 0, end: 2, left: 1 })); "up")]
#[test_case(1, false => Some((2, ListChange::Rotate { start: 1, end: 3, left: 1 })); "down")]
#[test_case(0, true => None; "first can't go up")]
#[test_case(2, false => None; "last can't go down")]
fn splits_move_one_place(index: usize, up: bool) -> Option<(usize, ListChange)> {
    // Act, with the expected value checked by test_case
    draft(&["A", "B", "C"]).move_split(index, up)
}

#[test]
fn other_lists_follow_the_splits() {
    // Arrange: what's been grabbed for each split, different for each, so every change shows
    let mut draft = draft(&["A", "B", "C"]);
    let mut grabbed = vec![Some("a"), Some("b"), Some("c")];

    // Act: C moves above B, A is duplicated, then B is removed
    for change in [draft.move_split(2, true).unwrap().1, draft.duplicate(0).unwrap().1, draft.remove(3).unwrap().1] {
        change.mirror(&mut grabbed);
    }

    // Assert: the copy has nothing grabbed yet
    assert_eq!((names(&draft), grabbed), (vec!["A", "A", "C"], vec![Some("a"), None, Some("c")]));
}

#[test_case(ListChange::Insert(1) => vec!["a", "", "b", "c"]; "insert")]
#[test_case(ListChange::Insert(9) => vec!["a", "b", "c", ""]; "insert past the end goes last")]
#[test_case(ListChange::Remove(1) => vec!["a", "c"]; "remove")]
#[test_case(ListChange::Remove(3) => vec!["a", "b", "c"]; "remove past the end does nothing")]
#[test_case(ListChange::Swap(0, 2) => vec!["c", "b", "a"]; "swap")]
#[test_case(ListChange::Swap(0, 3) => vec!["a", "b", "c"]; "swap past the end does nothing")]
#[test_case(ListChange::Rotate { start: 0, end: 3, left: 1 } => vec!["b", "c", "a"]; "rotate")]
#[test_case(ListChange::Rotate { start: 1, end: 4, left: 1 } => vec!["a", "b", "c"]; "rotate past the end does nothing")]
#[test_case(ListChange::Rotate { start: 1, end: 2, left: 2 } => vec!["a", "b", "c"]; "rotate by more than it has does nothing")]
fn changes_are_made_in_other_lists(change: ListChange) -> Vec<&'static str> {
    // Arrange
    let mut items = vec!["a", "b", "c"];

    // Act
    change.mirror(&mut items);

    // Assert, with the expected value checked by test_case
    items
}

#[test]
fn problems_name_their_split() {
    // Arrange
    let mut draft = draft(&["Boss", "Door", "Lift", "Lift", "reset"]);
    draft.preset.name = " ".into();
    draft.edit_split(0, SplitEdit::Mask(String::new()));
    // The same name as the split before, which it's compared together with
    draft.edit_split(3, SplitEdit::WithPrevious(true));
    draft.edit_split(1, SplitEdit::Name("Door (2)".into()));

    // Act
    let problems = draft.problems();

    // Assert
    let problem =
        |split: Option<usize>, field: Option<Field>, message: &str| Problem { split, field, message: message.into() };
    assert_eq!(
        problems,
        [
            problem(None, None, "The preset needs a name."),
            problem(Some(0), Some(Field::Mask), "It needs a mask. Choose one."),
            problem(
                Some(1),
                Some(Field::Name),
                "The name can't have '(', which AutoSplit reads as the start of a setting in file names."
            ),
            problem(
                Some(3),
                Some(Field::Name),
                "It has the same name as a split above it that it's compared together with. Give it another name."
            ),
        ]
    );
}

#[test_case("" => true; "none")]
#[test_case("  " => true; "only spaces")]
#[test_case("Game" => false; "one")]
fn a_preset_needs_a_game(game: &str) -> bool {
    // Arrange
    let mut draft = draft(&["Boss"]);
    draft.preset.game = game.into();

    // Act
    let problems = draft.problems();

    // Assert, with the expected value checked by test_case
    problems.iter().any(|problem| problem.message.starts_with("The preset needs a game."))
}

#[test]
fn a_preset_without_splits_needs_one() {
    // Arrange
    let mut draft = draft(&["Boss"]);
    draft.remove(0);

    // Act
    let problems = draft.problems();

    // Assert
    assert_eq!(problems, [Problem { split: None, field: None, message: "The preset needs a split. Add one.".into() }]);
}

#[test_case(1.3 => 1.3; "pause")]
#[test_case(0.1 => 0.1; "short pause")]
#[test_case(0.92 => 0.92; "threshold")]
#[test_case(60.0 => 60.0; "whole number")]
fn fields_give_the_number_they_show(value: f32) -> f64 {
    // Act, with the expected value checked by test_case
    field_value(value)
}

#[test]
fn a_pause_from_a_field_names_the_file_as_typed() {
    // Arrange
    let mut draft = draft(&["Boss"]);

    // Act
    draft.edit_split(0, SplitEdit::Pause(Some(field_value(1.3))));

    // Assert
    assert_eq!(draft.preset.file_names(None), ["0_Boss_[1.3].png"]);
}

/// The problems of each split, by index, without the preset's.
fn split_problems(draft: &Draft) -> Vec<(usize, String)> {
    draft.problems().into_iter().filter_map(|problem| Some((problem.split?, problem.message))).collect()
}

#[test]
fn up_to_four_reset_images_are_fine() {
    // Arrange
    let draft = draft(&["reset", "reset", "reset", "reset", "Boss"]);

    // Act
    let problems = draft.problems();

    // Assert
    assert_eq!(problems, []);
}

#[test]
fn every_reset_image_past_four_is_one_too_many() {
    // Arrange
    let draft = draft(&["reset", "reset", "reset", "reset", "reset", "reset", "Boss"]);

    // Act
    let problems = split_problems(&draft);

    // Assert
    assert_eq!(
        problems,
        [
            (4, "A preset can have up to 4 reset images.".to_owned()),
            (5, "A preset can have up to 4 reset images.".to_owned()),
        ]
    );
}

#[test]
fn up_to_four_splits_can_be_compared_together() {
    // Arrange
    let mut draft = draft(&["A", "B", "C", "D", "E"]);
    for index in 1..5 {
        draft.edit_split(index, SplitEdit::WithPrevious(true));
    }

    // Act
    let problems = split_problems(&draft);

    // Assert
    assert_eq!(problems, [(4, "Up to 4 splits can be compared together.".to_owned())]);
}

#[test_case(&["Boss", "Door"], 0 => vec![(0, "There's no split before it to compare it with.".to_owned())]; "first split")]
#[test_case(&["reset", "Door"], 1 => vec![(1, "There's no split before it to compare it with.".to_owned())]; "after a reset image")]
#[test_case(&["Boss", "reset"], 1 => vec![(1, "A reset image can't be compared with the split before it.".to_owned())]; "reset image")]
#[test_case(&["Boss", "Door"], 1 => Vec::<(usize, String)>::new(); "after a split")]
fn compared_with_the_split_before_needs_one(names: &[&str], index: usize) -> Vec<(usize, String)> {
    // Arrange
    let mut draft = draft(names);

    // Act
    draft.edit_split(index, SplitEdit::WithPrevious(true));

    // Assert, with the expected value checked by test_case
    split_problems(&draft)
}

#[test]
fn a_preset_of_reset_and_start_images_needs_a_split() {
    // Arrange
    let draft = draft(&["start_auto_splitter", "reset"]);

    // Act
    let problems = draft.problems();

    // Assert
    let message = "The preset needs a split besides the start and reset images.";
    assert_eq!(problems, [Problem { split: None, field: None, message: message.into() }]);
}

#[test_case(&["Boss", "Boss"], 1 => (true, true); "same name")]
#[test_case(&["Boss", "boss"], 1 => (true, true); "same name in another case")]
#[test_case(&["Bär", "BÄR"], 1 => (true, true); "same name in another case beyond ascii")]
#[test_case(&["Boss", "Door", "Boss"], 2 => (true, true); "same name as the first of three")]
#[test_case(&["Boss", "Boss Alt"], 1 => (false, false); "other name")]
fn splits_compared_together_need_their_own_names(names: &[&str], index: usize) -> (bool, bool) {
    // Arrange: every split compared together with the first
    let mut draft = draft(names);
    for joined in 1..names.len() {
        draft.edit_split(joined, SplitEdit::WithPrevious(true));
    }

    // Act
    let problems = split_problems(&draft);

    // Assert: a problem, so it can't be saved, and the Name field shows it
    (
        problems.iter().any(|(split, _)| *split == index),
        !field_problems(&draft.problems(), index, Field::Name).is_empty(),
    )
}

#[test]
fn the_same_name_is_fine_for_splits_not_compared_together() {
    // Arrange
    let draft = draft(&["Boss", "Boss"]);

    // Act
    let problems = draft.problems();

    // Assert
    assert_eq!((problems, field_problems(&draft.problems(), 1, Field::Name)), (Vec::new(), String::new()));
}

#[test]
fn bundled_presets_have_nothing_to_fix() {
    // Arrange
    let presets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../presets");
    let (loaded, errors) = autosplit_core::preset::load_presets(&presets);

    // Act: a preset with problems can't be used to make split images
    let problems: Vec<(String, Vec<Problem>)> = loaded
        .into_iter()
        .map(|loaded| (loaded.preset.name.clone(), Draft::edit(loaded.preset).problems()))
        .filter(|(_, problems)| !problems.is_empty())
        .collect();

    // Assert
    assert_eq!((errors.len(), problems), (0, Vec::new()));
}

/// A draft of splits named `names`, where a `+` in front means compared with the split above.
fn grouped(names: &[&str]) -> Draft {
    let mut draft = draft(&names.iter().map(|name| name.trim_start_matches('+')).collect::<Vec<_>>());
    for (index, name) in names.iter().enumerate() {
        draft.edit_split(index, SplitEdit::WithPrevious(name.starts_with('+')));
    }
    draft
}

/// The splits' names, with `+` in front of the ones compared with the split above.
fn grouped_names(draft: &Draft) -> String {
    let preset = &draft.preset;
    (preset.splits.iter().enumerate())
        .map(|(index, split)| format!("{}{}", if preset.joins_previous(index) { "+" } else { "" }, split.name))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A preset whose split `A` is still ticked to be compared with the split above after the split
/// above, `X`, became a reset image.
fn ticked_under_a_reset_image() -> Draft {
    let mut draft = grouped(&["Y", "X", "+A", "B"]);
    draft.edit_split(1, SplitEdit::Kind(SplitKind::Reset));
    draft
}

#[test_case(|draft| draft.move_split(3, true).map(|_| ()) => "Y reset B A".to_owned(); "a split moved above it")]
#[test_case(|draft| draft.remove(1).map(|_| ()) => "Y A B".to_owned(); "the reset image removed")]
#[test_case(|draft| Some(draft.add(Some(1))).map(|_| ()) => "Y reset Split1 A B".to_owned(); "a split added above it")]
#[test_case(|draft| draft.duplicate(2).map(|_| ()) => "Y reset A A B".to_owned(); "a copy of it")]
fn list_changes_never_compare_a_split_with_another_by_surprise(change: fn(&mut Draft) -> Option<()>) -> String {
    // Arrange
    let mut draft = ticked_under_a_reset_image();

    // Act
    change(&mut draft).unwrap();

    // Assert, with the expected value checked by test_case
    grouped_names(&draft)
}

#[test]
fn removing_the_first_of_a_group_leaves_the_rest_together() {
    // Arrange
    let mut draft = grouped(&["X", "A", "+B", "+C"]);

    // Act
    draft.remove(1);

    // Assert: B isn't compared with X
    assert_eq!(grouped_names(&draft), "X B +C");
}

#[test]
fn added_splits_go_after_the_group() {
    // Arrange
    let mut draft = grouped(&["A", "+B", "C"]);

    // Act
    let added = draft.add(Some(0));

    // Assert
    assert_eq!((added, grouped_names(&draft)), ((2, ListChange::Insert(2)), "A +B Split1 C".to_owned()));
}

#[test_case(0 => (2, "A +B A C".to_owned()); "the first is a split of its own")]
#[test_case(1 => (2, "A +B +B C".to_owned()); "the others join the group")]
fn copies_go_after_the_group(index: usize) -> (usize, String) {
    // Arrange
    let mut draft = grouped(&["A", "+B", "C"]);

    // Act
    let (at, _) = draft.duplicate(index).unwrap();

    // Assert, with the expected value checked by test_case
    (at, grouped_names(&draft))
}

#[test_case(&["X", "A", "+B"], 2, true => (1, "X B +A".to_owned()); "inside the group")]
#[test_case(&["X", "A", "+B"], 1, false => (2, "X B +A".to_owned()); "the first down inside the group")]
#[test_case(&["X", "A", "+B"], 1, true => (0, "A +B X".to_owned()); "the group up")]
#[test_case(&["A", "+B", "X"], 1, false => (2, "X A +B".to_owned()); "the group down")]
#[test_case(&["X", "A", "+B", "Y"], 0, false => (2, "A +B X Y".to_owned()); "past a group")]
fn groups_move_together(names: &[&str], index: usize, up: bool) -> (usize, String) {
    // Arrange
    let mut draft = grouped(names);

    // Act
    let (to, _) = draft.move_split(index, up).unwrap();

    // Assert, with the expected value checked by test_case
    (to, grouped_names(&draft))
}

#[test]
fn other_lists_follow_groups_that_move() {
    // Arrange
    let mut draft = grouped(&["X", "A", "+B"]);
    let mut grabbed = vec![Some("x"), Some("a"), None];

    // Act
    draft.move_split(1, true).unwrap().1.mirror(&mut grabbed);

    // Assert
    assert_eq!(grabbed, [Some("a"), None, Some("x")]);
}

#[test]
fn an_untouched_new_preset_has_no_changes_but_can_be_saved() {
    // Arrange
    let mut draft = Draft::new("Game");

    // Act
    let untouched = (draft.has_changes(), draft.can_save());
    draft.preset.name = "Any%".into();

    // Assert: closing it doesn't ask, and an edit counts
    assert_eq!((untouched, draft.has_changes()), ((false, true), true));
}

#[test]
fn problems_show_by_the_field_they_are_about() {
    // Arrange: no mask, an invalid name, and compared with nothing above
    let mut draft = draft(&["Boss"]);
    draft.edit_split(0, SplitEdit::Mask(String::new()));
    draft.edit_split(0, SplitEdit::Name("Boss {".into()));
    draft.edit_split(0, SplitEdit::WithPrevious(true));

    // Act
    let fields =
        [Field::Name, Field::Mask, Field::Group].map(|field| !field_problems(&draft.problems(), 0, field).is_empty());

    // Assert
    assert_eq!(fields, [true, true, true]);
}

#[test_case(SplitKind::Reset => ("reset".to_owned(), false); "reset image")]
#[test_case(SplitKind::Start => ("start_auto_splitter".to_owned(), false); "start image")]
#[test_case(SplitKind::Split => ("Door".to_owned(), true); "already a split")]
fn the_kind_sets_the_name(kind: SplitKind) -> (String, bool) {
    // Arrange: a split compared with the one above
    let mut draft = grouped(&["Boss", "+Door"]);

    // Act
    draft.edit_split(1, SplitEdit::Kind(kind));

    // Assert, with the expected value checked by test_case: start and reset images aren't
    let split = &draft.preset.splits[1];
    (split.name.clone(), split.with_previous)
}

#[test]
fn a_reset_image_made_a_split_gets_a_name_of_its_own() {
    // Arrange
    let mut draft = draft(&["reset", "Split1"]);

    // Act
    draft.edit_split(0, SplitEdit::Kind(SplitKind::Split));

    // Assert
    assert_eq!((names(&draft), SplitKind::of(&draft.preset.splits[0])), (vec!["Split2", "Split1"], SplitKind::Split));
}

#[test]
fn savestate_extensions_are_the_presets_own() {
    // Arrange: two kinds, one twice and in another case, and a split without one
    let mut draft = draft(&["A", "B", "C", "D"]);
    for (index, savestate) in ["a.abc", "", "b.EFG", "c.abc"].into_iter().enumerate() {
        draft.edit_split(index, SplitEdit::Savestate(savestate.into()));
    }

    // Act
    let extensions = draft.savestate_extensions();

    // Assert
    assert_eq!(extensions, ["abc", "efg"]);
}

#[test_case(&["Boss", "Door"], 1 => None; "after a split")]
#[test_case(&["Boss"], 0 => Some("It's the first split, so there's no split above to compare it with."); "first")]
#[test_case(&["Boss", "reset"], 1 => Some("Start and reset images aren't splits, so they can't be compared with one."); "reset image")]
#[test_case(&["reset", "Door"], 1 => Some("The split above is a start or reset image, which isn't a split to compare it with."); "after a reset image")]
#[test_case(&["A", "+B", "+C", "+D", "E"], 4 => Some("The splits above are already 4 compared together, the most there can be."); "after a full group")]
#[test_case(&["A", "+B", "+C", "D"], 3 => None; "after a group with room")]
fn the_checkbox_says_why_it_cant_join(names: &[&str], index: usize) -> Option<&'static str> {
    // Act, with the expected value checked by test_case
    grouped(names).why_not_joined(index)
}

#[test_case(0 => "Reset Image"; "a reset image")]
#[test_case(1 => "Reset Image 2"; "the second reset image")]
#[test_case(2 => "Start Image"; "the start image")]
#[test_case(3 => "Boss"; "a split")]
fn headings_name_reset_and_start_images(index: usize) -> String {
    // Act, with the expected value checked by test_case
    split_heading(&draft(&["reset", "reset", "start_auto_splitter", "Boss"]).preset, index)
}

#[test]
fn the_summary_names_the_first_problem_and_counts_the_rest() {
    // Arrange: no mask on either split, and a preset without a name
    let mut draft = draft(&["Boss", "Door"]);
    draft.preset.name = String::new();
    draft.edit_split(0, SplitEdit::Mask(String::new()));
    draft.edit_split(1, SplitEdit::Mask(String::new()));
    let problems = draft.problems();

    // Act
    let summary = problem_summary(&draft.preset, &problems);

    // Assert
    assert_eq!(summary.as_deref(), Some("The preset needs a name. And 2 more problems."));
}

#[test_case(false => "No changes yet."; "unchanged")]
#[test_case(true => "Ready to save."; "changed")]
fn without_problems_the_line_says_whether_theres_something_to_save(changed: bool) -> String {
    // Arrange
    let mut draft = draft(&["Boss"]);
    if changed {
        draft.edit_split(0, SplitEdit::Name("Door".into()));
    }

    // Act, with the expected value checked by test_case
    status_line(&draft.preset, &draft.problems(), draft.has_changes())
}

#[test]
fn with_problems_the_line_is_their_summary() {
    // Arrange
    let mut draft = draft(&["Boss"]);
    draft.edit_split(0, SplitEdit::Mask(String::new()));
    let problems = draft.problems();

    // Act
    let line = status_line(&draft.preset, &problems, draft.has_changes());

    // Assert
    assert_eq!(Some(line), problem_summary(&draft.preset, &problems));
}

#[test_case(0 => Some(2); "the next one")]
#[test_case(2 => Some(3); "after the selected one")]
#[test_case(3 => Some(0); "from the top after the last")]
fn the_problem_line_goes_to_the_next_split_with_a_problem(selected: usize) -> Option<usize> {
    // Arrange: splits 0, 2 and 3 have no mask
    let mut draft = draft(&["A", "B", "C", "D"]);
    for index in [0, 2, 3] {
        draft.edit_split(index, SplitEdit::Mask(String::new()));
    }

    // Act, with the expected value checked by test_case
    next_problem_split(&draft.problems(), selected)
}

#[test]
fn without_problems_the_line_goes_nowhere() {
    assert_eq!(next_problem_split(&draft(&["A"]).problems(), 0), None);
}

#[test]
fn the_list_has_every_problem_with_its_split() {
    // Arrange
    let mut draft = draft(&["Boss", "Door"]);
    draft.edit_split(1, SplitEdit::Mask(String::new()));
    let problems = draft.problems();

    // Act
    let list = problem_list(&draft.preset, &problems);
    let flags = problem_flags(&problems, 2);

    // Assert
    assert_eq!((list.as_str(), flags), ("- Door: It needs a mask. Choose one.", vec![false, true]));
}

#[test_case(Some(1000.4) => Some(1000); "rounded")]
#[test_case(Some(-5.0) => Some(0); "never below zero")]
#[test_case(None => None; "the profile's")]
fn delays_are_whole_milliseconds(delay: Option<f64>) -> Option<u32> {
    // Arrange
    let mut draft = draft(&["Boss"]);

    // Act
    draft.edit_split(0, SplitEdit::Delay(delay));

    // Assert, with the expected value checked by test_case
    draft.preset.splits[0].delay
}

#[test]
fn only_the_type_changes_other_fields() {
    // Arrange
    let edits = [SplitEdit::Kind(SplitKind::Reset), SplitEdit::Name("Boss".into()), SplitEdit::Mask("a.png".into())];

    // Act
    let flags: Vec<(bool, bool)> =
        edits.iter().map(|edit| (edit.changes_other_fields(), edit.changes_mask())).collect();

    // Assert
    assert_eq!(flags, [(true, false), (false, false), (false, true)]);
}

#[test_case("the legend of zelda:  ocarina of time " => Some("The Legend of Zelda: Ocarina of Time"); "other case and spaces")]
#[test_case("The Legend of Zelda: Majora's Mask" => None; "another game")]
#[test_case("" => None; "no game")]
fn typed_games_are_the_known_ones(game: &str) -> Option<&'static str> {
    // Act, with the expected value checked by test_case
    known_game(game, &["", "The Legend of Zelda: Ocarina of Time", "Super Mario 64"])
}

#[test_case("No Wrong Warp", true => ("Discard the changes to No Wrong Warp?".to_owned(), "Discard Changes"); "saved before")]
#[test_case(" ", true => ("Discard the changes to this preset?".to_owned(), "Discard Changes"); "saved before, name cleared")]
#[test_case("Any%", false => ("Discard the new preset Any%? It hasn't been saved yet.".to_owned(), "Discard Preset"); "new")]
#[test_case("", false => ("Discard the new preset? It hasn't been saved yet.".to_owned(), "Discard Preset"); "new without a name")]
fn leaving_unsaved_changes_says_what_is_lost(name: &str, saved_before: bool) -> (String, &'static str) {
    // Act, with the expected value checked by test_case
    discard_question(name, saved_before)
}

#[test_case("006 enter deku.gzs", 15 => "006 en...ku.gzs"; "shortened in the middle")]
#[test_case("deku.gzs", 15 => "deku.gzs"; "short enough")]
#[test_case("006 enter deku.gzs", 18 => "006 enter deku.gzs"; "exactly the limit")]
#[test_case("Dékù Trée.gzs", 12 => "Dékù...e.gzs"; "characters, not bytes")]
fn long_file_names_keep_their_start_and_end(name: &str, max: usize) -> String {
    // Act, with the expected value checked by test_case
    elide_middle(name, max)
}

fn library_split(name: &str) -> PresetSplit {
    PresetSplit {
        name: name.into(),
        mask: "escape.png".into(),
        threshold: Some(0.97),
        delay: Some(150),
        with_previous: true,
        ..PresetSplit::default()
    }
}

#[test]
fn imported_splits_go_after_the_selected_one_with_their_files_in_the_library() {
    // Arrange
    let mut draft = draft(&["Boss", "Door"]);
    let library = std::env::temp_dir().join("splits").join("Game");

    // Act
    let imported = draft.import(draft.place_after(Some(0)), &[library_split("Escape")], &library);

    // Assert: settings kept as the library has them, a split of its own
    let expected = PresetSplit {
        mask: library.join("escape.png").display().to_string(),
        with_previous: false,
        ..library_split("Escape")
    };
    assert_eq!((imported, &draft.preset.splits[1]), (vec![ListChange::Insert(1)], &expected));
}

#[test]
fn imported_savestates_point_into_the_library_too() {
    // Arrange
    let mut draft = draft(&["Boss"]);
    let library = std::env::temp_dir().join("splits").join("Game");
    let split = PresetSplit { savestate: "escape.gzs".into(), ..library_split("Escape") };

    // Act
    draft.import(draft.place_after(None), &[split], &library);

    // Assert
    assert_eq!(draft.preset.splits[1].savestate, library.join("escape.gzs").display().to_string());
}

#[test]
fn imported_splits_go_after_the_splits_compared_with_the_selected_one() {
    // Arrange
    let mut draft = draft(&["Boss", "Boss 2", "Door"]);
    draft.preset.splits[1].with_previous = true;

    // Act
    let place = draft.place_after(Some(0));
    draft.import(place, &[library_split("Escape")], std::path::Path::new("library"));

    // Assert
    assert_eq!((place, names(&draft)), (2, vec!["Boss", "Boss 2", "Escape", "Door"]));
}

#[test_case("reset" => "Reset Image"; "reset image")]
#[test_case("start_auto_splitter" => "Start Image"; "start image")]
#[test_case("Escape" => "Escape"; "a split")]
fn library_splits_are_headed_like_the_split_list(name: &str) -> String {
    // Act, with the expected value checked by test_case
    library_heading(&library_split(name))
}

#[test_case("" => true; "an empty search")]
#[test_case("ganon" => true; "a name in another case")]
#[test_case("slash  INTRO" => true; "words in different texts")]
#[test_case("ganon bottle" => false; "a word in none")]
#[test_case("flash" => false; "nothing has it")]
fn searching_needs_every_word_somewhere(search: &str) -> bool {
    // Act, with the expected value checked by test_case
    matches_search(&["Ganon", "Ganon intro", "Final slash on Ganon"], search)
}

#[test_case(&["reset", "reset", "reset"], &["reset"] => None; "room for another")]
#[test_case(&["reset", "reset", "reset", "reset"], &["reset"] => Some("A preset can have up to 4 reset images, and this would make 5.".to_owned()); "reset images full")]
#[test_case(&["reset", "reset", "reset"], &["reset", "reset"] => Some("A preset can have up to 4 reset images, and this would make 5.".to_owned()); "steps past the most")]
#[test_case(&["reset", "reset", "reset", "reset"], &["start_auto_splitter"] => None; "another kind")]
#[test_case(&["Escape", "Escape", "Escape", "Escape"], &["Escape"] => None; "splits have no limit")]
fn importing_stops_at_the_most_reset_or_start_images(names: &[&str], steps: &[&str]) -> Option<String> {
    // Arrange
    let preset = draft(names).preset;
    let steps: Vec<PresetSplit> = steps.iter().map(|name| library_split(name)).collect();

    // Act
    import_blocker(&preset, &steps)
}

#[test]
fn ready_made_splits_import_all_their_steps_in_order() {
    // Arrange: a dummy for the intro text, then the flash, the flash also matching a second look
    let mut draft = draft(&["Boss", "Door"]);
    let intro = PresetSplit { dummy: true, ..library_split("Ganon intro") };
    let flash = PresetSplit { with_previous: false, ..library_split("Ganon") };
    let other_flash = library_split("Ganon");

    // Act
    let changes = draft.import(1, &[intro, flash, other_flash], std::path::Path::new("library"));

    // Assert: the first isn't compared with Boss, the last still with the flash before it
    let joined: Vec<bool> = draft.preset.splits.iter().map(|split| split.with_previous).collect();
    assert_eq!(
        (changes, names(&draft), joined),
        (
            vec![ListChange::Insert(1), ListChange::Insert(2), ListChange::Insert(3)],
            vec!["Boss", "Ganon intro", "Ganon", "Ganon", "Door"],
            vec![false, false, false, true, false]
        )
    );
}

#[test_case(Some(2) => (1, 3); "a split compared with the one above")]
#[test_case(Some(0) => (0, 1); "a split of its own")]
#[test_case(None => (4, 4); "nothing selected")]
fn inserting_above_or_below_goes_around_splits_compared_together(selected: Option<usize>) -> (usize, usize) {
    // Arrange
    let draft = grouped(&["Door", "Boss", "+Boss 2", "Exit"]);

    // Act
    (draft.place_before(selected), draft.place_after(selected))
}

#[test_case(0 => 0; "above the first")]
#[test_case(1 => 1; "between splits of their own")]
#[test_case(2 => 1; "in the middle of a group of two, its start")]
#[test_case(4 => 4; "at the end")]
#[test_case(9 => 4; "past the end")]
fn drops_never_go_between_splits_compared_together(gap: usize) -> usize {
    // Act, with the expected value checked by test_case
    grouped(&["Door", "Boss", "+Boss 2", "Exit"]).drop_place(gap)
}

#[test]
fn drops_inside_a_group_go_to_its_nearer_end() {
    // Arrange: a group of three, so its second gap is nearer the end
    let draft = grouped(&["Boss", "+Boss 2", "+Boss 3", "Exit"]);

    // Act
    let places = (draft.drop_place(1), draft.drop_place(2));

    // Assert
    assert_eq!(places, (0, 3));
}

#[test_case("Boss" => Some(("2_Boss".to_owned(), "Already in the preset, as split 2.".to_owned())); "a split, numbered like its file")]
#[test_case("reset" => Some(("reset".to_owned(), "Already in the preset, as its reset image.".to_owned())); "a reset image")]
#[test_case("Door" => None; "not in it")]
fn splits_in_the_preset_say_where(name: &str) -> Option<(String, String)> {
    // Arrange: file names without the mask's extension, which the draft's splits have none of
    let preset = draft(&["reset", "Intro", "Boss"]).preset;

    // Act
    in_preset(&preset, name, None).map(|found| (found.file.trim_end_matches(".png").to_owned(), found.note))
}

#[test_case("Escape" => "XX_Escape_(0.97)_#150#.png"; "a split, numbered by where it's put")]
#[test_case("reset" => "reset_(0.97)_#150#.png"; "a reset image, which has no number")]
fn ready_made_splits_name_their_files_without_a_place(name: &str) -> String {
    // Act, with the expected value checked by test_case
    library_file_name(&library_split(name), None)
}

#[test_case(Some(1), 1, 2, true => 3; "above the selected split, which stays selected")]
#[test_case(Some(1), 2, 2, false => 3; "below it, the last new one")]
#[test_case(None, 3, 1, true => 4; "above with none selected, still none")]
#[test_case(None, 3, 1, false => 3; "below with none selected, the new one")]
fn inserted_splits_keep_the_order_they_are_put_in(
    selected: Option<usize>,
    at: usize,
    count: usize,
    above: bool,
) -> usize {
    // Arrange: the draft with the new splits in
    let mut draft = draft(&["A", "B", "C"]);
    for n in 0..count {
        draft.preset.splits.insert(at + n, split("New"));
    }

    // Act
    draft.selected_after_import(selected, at, count, above)
}

#[test]
fn inserting_above_with_nothing_selected_keeps_click_order() {
    // Arrange: an empty preset, so nothing is selected
    let mut draft = draft(&[]);
    let mut selected = None;

    // Act: Insert Above A, then B, as the window does
    for name in ["A", "B"] {
        let at = draft.place_before(selected);
        draft.import(at, &[split(name)], std::path::Path::new("library"));
        let select = draft.selected_after_import(selected, at, 1, true);
        selected = (select < draft.preset.splits.len()).then_some(select);
    }

    // Assert
    assert_eq!(names(&draft), ["A", "B"]);
}

#[test]
fn splits_in_the_preset_are_named_like_the_split_list() {
    // Arrange: a profile that compares with histograms, which the split list's names say for
    // splits that keep the L2 norm
    let preset = draft(&["Intro", "Boss"]).preset;
    let defaults = autosplit_core::split_image::SplitDefaults {
        comparison_method: 1,
        similarity_threshold: 0.95,
        delay_time: 0,
        pause_time: 10.0,
        fps_limit: 60,
        fade_tolerance: 10,
    };

    // Act
    let shown = in_preset(&preset, "Boss", Some(&defaults)).map(|found| found.file);

    // Assert
    assert_eq!(shown, preset.file_names(Some(&defaults)).get(1).cloned());
}
