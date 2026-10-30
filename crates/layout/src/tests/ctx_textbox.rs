use crate::textbox::probe_work_tests;

#[test]
fn nested_none_is_not_cached() {
    probe_work_tests::nested_none_is_not_cached(|ctx, p| ctx.fitted_boxes(p));
}

#[test]
fn floating_inline_state_changes() {
    probe_work_tests::floating_inline_state_changes(|ctx, p| ctx.fitted_boxes(p));
}
