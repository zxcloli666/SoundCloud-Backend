use super::off_the_track;

#[test]
fn a_file_short_of_nine_tenths_of_the_track_is_off_even_inside_a_wide_tolerance() {
    assert!(off_the_track(200.0, 170.0, 60.0));
    assert!(off_the_track(219.246, 20.0, 300.0));
}

#[test]
fn a_full_file_inside_the_tolerance_passes() {
    assert!(!off_the_track(200.0, 185.0, 60.0));
    assert!(!off_the_track(200.0, 199.2, 2.0));
    assert!(!off_the_track(200.0, 200.9, 2.0));
}

#[test]
fn the_tolerance_still_holds_for_long_and_short_drifts() {
    assert!(off_the_track(200.0, 205.0, 2.0));
    assert!(off_the_track(200.0, 195.0, 2.0));
}
