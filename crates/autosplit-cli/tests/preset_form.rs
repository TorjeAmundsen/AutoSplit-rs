//! The preset editor's draft: edits, list operations kept in step with other lists, and problems.

use autosplit::gui::preset_form::{Draft, Field, ListChange, Problem, SplitEdit, SplitKind, field_value};
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

#[test_case(0.95 => None; "default isn't written")]
#[test_case(0.9 => Some(0.9); "own threshold")]
#[test_case(0.934 => Some(0.93); "rounded like the spin box")]
#[test_case(1.5 => Some(1.0); "at most 1")]
fn thresholds_leave_the_default_out(threshold: f64) -> Option<f64> {
    // Arrange
    let mut draft = draft(&["Boss"]);

    // Act
    draft.edit_split(0, SplitEdit::Threshold(threshold));

    // Assert, with the expected value checked by test_case
    draft.preset.splits[0].threshold
}

#[test]
fn clearing_the_savestate_clears_its_instructions() {
    // Arrange
    let mut draft = draft(&["Boss"]);
    draft.edit_split(0, SplitEdit::Savestate("boss.gzs".into()));
    draft.edit_split(0, SplitEdit::Instructions("Advance a frame.".into()));

    // Act
    draft.edit_split(0, SplitEdit::Savestate(String::new()));

    // Assert
    assert_eq!(draft.preset.splits[0].instructions, "");
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

#[test_case(&["Boss", "Door"], 1 => Some((Some(0), ListChange::Remove(1))); "last selects the one before")]
#[test_case(&["Boss", "Door"], 0 => Some((Some(0), ListChange::Remove(0))); "first selects the next")]
#[test_case(&["Boss"], 0 => Some((None, ListChange::Remove(0))); "only split leaves nothing selected")]
#[test_case(&["Boss"], 3 => None; "no such split")]
fn removing_selects_a_neighbor(splits: &[&str], index: usize) -> Option<(Option<usize>, ListChange)> {
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
    // Arrange: what's been grabbed for each split
    let mut draft = draft(&["A", "B", "C"]);
    let mut grabbed = vec![Some("a"), None, Some("c")];

    // Act
    for change in [draft.move_split(2, true).unwrap().1, draft.duplicate(0).unwrap().1, draft.remove(3).unwrap().1] {
        change.mirror(&mut grabbed);
    }

    // Assert: the copy has nothing grabbed yet
    assert_eq!((names(&draft), grabbed), (vec!["A", "A", "C"], vec![Some("a"), None, Some("c")]));
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
                "It has the same name as a split it's compared together with. Give it another name."
            ),
        ]
    );
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
    assert_eq!(draft.preset.file_names(None), ["0_Boss_(0.95)_[1.3].png"]);
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
    let message = "The preset needs a split besides the start_auto_splitter and reset images.";
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
    (problems.iter().any(|(split, _)| *split == index), !draft.field_problems(index, Field::Name).is_empty())
}

#[test]
fn the_same_name_is_fine_for_splits_not_compared_together() {
    // Arrange
    let draft = draft(&["Boss", "Boss"]);

    // Act
    let problems = draft.problems();

    // Assert
    assert_eq!((problems, draft.field_problems(1, Field::Name)), (Vec::new(), String::new()));
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
    let fields = [Field::Name, Field::Mask, Field::Group].map(|field| !draft.field_problems(0, field).is_empty());

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
