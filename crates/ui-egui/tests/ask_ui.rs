#![allow(clippy::unwrap_used)]

use egui::{Context, RawInput, Rect, pos2};
use wordcraft_ui_egui::ask_agent::{Snapshot, Status, Turn, ui};

fn anchor() -> Rect {
    Rect::from_min_max(pos2(20.0, 20.0), pos2(100.0, 40.0))
}

#[test]
/// Checks that instructions replies and progress are rendered as literal text.
fn instructions_replies_and_progress_are_rendered_as_literal_text() {
    let ctx = Context::default();
    let answer = "<script>alert('x')</script> [click](https://invalid.example) **bold**";
    let instruction = "<b>instruction</b>";
    let progress = "<img src=x onerror=alert(1)>";
    let proposal = "{\"tool\":\"delete_document\"}";
    let comment = "<a href=file:///secret>review note</a>";
    let snapshot = Snapshot {
        id: 1,
        status: Status::Answered,
        agent: None,
        notes: vec![progress.into()],
        answer: Some(answer.into()),
        error: None,
        proposal: Some(proposal.into()),
        format: Some("json".into()),
        comment: Some(comment.into()),
    };
    let history = [Turn { instruction: "[old](file:///secret)".into(), answer: "<a href=x>old</a>".into() }];
    let view = ui::View { snapshot: Some(&snapshot), agents: &[], actions: &[], history: &history, instruction };
    let mut draft = String::new();
    let mut picked = None;
    // egui can take an initial sizing pass before emitting visible text shapes.
    for _ in 0..3 {
        let mut output = ctx.run_pass(RawInput::default(), |ctx| {
            ui::bubble(ctx, anchor(), &view, &mut draft, &mut picked, &str::to_string);
        });
        output.textures_delta.clear();
        let text: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                _ => None,
            })
            .collect();
        if text.contains(&answer) {
            assert!(text.contains(&instruction));
            assert!(text.contains(&progress));
            assert!(text.contains(&proposal));
            assert!(text.contains(&comment));
            assert!(text.contains(&"Proposal (not applied)"));
            assert!(text.contains(&history[0].instruction.as_str()));
            assert!(text.contains(&history[0].answer.as_str()));
            assert!(output.platform_output.commands.is_empty(), "text must not launch URLs or copy commands");
            return;
        }
    }
    panic!("literal reply text was not rendered");
}

#[test]
/// Checks that an idle bubble does not steal focus after opening.
fn an_idle_bubble_does_not_steal_focus_after_opening() {
    let ctx = Context::default();
    let view = ui::View { snapshot: None, agents: &[], actions: &[], history: &[], instruction: "" };
    let mut draft = String::new();
    let mut picked = None;
    ctx.run_pass(RawInput::default(), |ctx| {
        ui::bubble(ctx, anchor(), &view, &mut draft, &mut picked, &str::to_string);
    })
    .drop_without_applying_deltas();
    let mut other = None;
    ctx.run_pass(RawInput::default(), |ctx| {
        ctx.root_ui(|ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let response = ui.button("Another control");
                response.request_focus();
                other = Some(response.id);
            });
        });
        ui::bubble(ctx, anchor(), &view, &mut draft, &mut picked, &str::to_string);
    })
    .drop_without_applying_deltas();
    assert_eq!(ctx.memory(|memory| memory.focused()), other);
}

#[test]
/// Checks that enter sends without inserting a newline and shift enter only inserts one.
fn enter_sends_without_inserting_a_newline_and_shift_enter_only_inserts_one() {
    for shift in [false, true] {
        let ctx = Context::default();
        let view = ui::View { snapshot: None, agents: &[], actions: &[], history: &[], instruction: "" };
        let mut draft = "abcdef".to_string();
        let mut picked = None;
        ctx.run_pass(RawInput::default(), |ctx| {
            ui::bubble(ctx, anchor(), &view, &mut draft, &mut picked, &str::to_string);
        })
        .drop_without_applying_deltas();
        let field = ctx.memory(|memory| memory.focused()).unwrap();
        let mut state = egui::text_edit::TextEditState::load(&ctx, field).unwrap();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(3))));
        state.store(&ctx, field);
        let modifiers = if shift { egui::Modifiers::SHIFT } else { egui::Modifiers::NONE };
        let input = RawInput {
            events: vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers },
            ],
            ..RawInput::default()
        };
        let mut action = None;
        ctx.run_pass(input, |ctx| {
            action = ui::bubble(ctx, anchor(), &view, &mut draft, &mut picked, &str::to_string);
        })
        .drop_without_applying_deltas();
        if shift {
            assert!(action.is_none());
            assert_eq!(draft, "abc\ndef");
        } else {
            assert_eq!(action, Some(ui::Action::Send("abcdef".into())));
            assert!(draft.is_empty());
        }
    }
}

#[test]
fn a_failed_request_keeps_literal_error_and_exposes_both_review_actions() {
    use serde_json::json;
    use wordcraft_ui_egui::ask_agent::{AgentHub, Reply, Request, Target};
    let hub = AgentHub::default();
    let target = Target { document: "synthetic:1".into(), revision: 2 };
    let id = hub
        .ask(Request {
            id: 0,
            instruction: "Review partial tool edits".into(),
            history: vec![],
            selection: json!({"pages":[1]}),
            document: json!({"id":"synthetic:1"}),
            agent: None,
            target: target.clone(),
        })
        .unwrap();
    hub.listen(1, Some("worker"), std::time::Duration::ZERO, || true).unwrap().unwrap();
    let error = "Partial edit failed: <script>literal error</script>";
    let proposal = "{\"tool\":\"delete_document\"}";
    hub.reply(1, id, Reply { error: Some(error), proposal: Some(proposal), format: Some("json"), ..Default::default() }, Some(&target)).unwrap();
    let snapshot = hub.snapshot(id).unwrap();
    assert_eq!(snapshot.status, Status::Failed);
    assert!(snapshot.review_text().contains(error));
    assert!(snapshot.review_text().contains(proposal));
    assert!(snapshot.review_text().contains("Proposal (not applied)"));
    for (label, expected) in [("Keep", ui::Action::Keep), ("Undo", ui::Action::Undo)] {
        let ctx = Context::default();
        let view = ui::View { snapshot: Some(&snapshot), agents: &[], actions: &[], history: &[], instruction: "" };
        let mut draft = String::new();
        let mut picked = None;
        let mut button = None;
        for _ in 0..3 {
            let output = ctx.run_pass(RawInput::default(), |ctx| {
                ui::bubble(ctx, anchor(), &view, &mut draft, &mut picked, &str::to_string);
            });
            for shape in &output.shapes {
                if let egui::epaint::Shape::Text(text) = &shape.shape
                    && text.galley.job.text == label
                {
                    button = Some(text.pos + text.galley.size() * 0.5);
                }
            }
            assert!(output.platform_output.commands.is_empty(), "an error/proposal must not execute a command");
            output.drop_without_applying_deltas();
        }
        let position = button.unwrap();
        let mut actual = None;
        for pressed in [true, false] {
            let input = RawInput {
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton { pos: position, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE },
                ],
                ..Default::default()
            };
            ctx.run_pass(input, |ctx| {
                if let Some(action) = ui::bubble(ctx, anchor(), &view, &mut draft, &mut picked, &str::to_string) {
                    actual = Some(action);
                }
            })
            .drop_without_applying_deltas();
        }
        assert_eq!(actual, Some(expected), "{label} must dispatch its review action on Failed");
    }
    assert_eq!(hub.snapshot(id).unwrap().status, Status::Failed, "rendering/choosing an action does not execute any proposal in the hub");
}

/// Payloads accepted by the real hub must leave review and follow-up actions reachable.
/// Exercise wheel input, actual clipped text bounds and button clicks for both terminal states.
#[test]
fn long_accepted_replies_keep_review_and_send_actions_reachable_by_wheel() {
    use serde_json::json;
    use wordcraft_ui_egui::ask_agent::{AgentHub, Reply, Request, Target};
    for failed in [false, true] {
        let hub = AgentHub::default();
        let target = Target { document: "synthetic:scroll".into(), revision: 1 };
        let id = hub
            .ask(Request {
                id: 0,
                instruction: "Review this synthetic selection".into(),
                history: vec![],
                selection: json!({"pages":[1]}),
                document: json!({}),
                target: target.clone(),
                agent: None,
            })
            .unwrap();
        hub.listen(1, None, std::time::Duration::ZERO, || true).unwrap().unwrap();
        let progress = "Progress: checking this synthetic selection.\n".repeat(80);
        for _ in 0..2 {
            hub.reply(1, id, Reply { text: &progress, done: false, ..Default::default() }, Some(&target)).unwrap();
        }
        let answer = "Review result: preserve the person's document.\n".repeat(60);
        let proposal = "Unapplied proposal: inspect this synthetic selection.\n".repeat(100);
        let reply = if failed {
            Reply { error: Some(&answer), proposal: Some(&proposal), ..Default::default() }
        } else {
            Reply { text: &answer, proposal: Some(&proposal), ..Default::default() }
        };
        hub.reply(1, id, reply, Some(&target)).unwrap();
        let snapshot = hub.snapshot(id).unwrap();
        assert_eq!(snapshot.notes, [progress.clone(), progress]);
        assert_eq!(snapshot.proposal.as_deref(), Some(proposal.as_str()));
        assert_eq!(if failed { snapshot.error.as_deref() } else { snapshot.answer.as_deref() }, Some(answer.as_str()));
        for (label, expected) in [("Keep", ui::Action::Keep), ("Undo", ui::Action::Undo), ("Send", ui::Action::Send("Follow up".into()))] {
            let ctx = Context::default();
            let screen = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(640.0, 480.0));
            let view = ui::View { snapshot: Some(&snapshot), agents: &[], actions: &[], history: &[], instruction: "" };
            let mut draft = "Follow up".to_owned();
            let mut picked = None;
            let mut tick = 0_u32;
            let mut frame = |events| {
                tick += 1;
                let input = RawInput { screen_rect: Some(screen), time: Some(f64::from(tick) / 30.0), events, ..Default::default() };
                let mut action = None;
                let output = ctx.run_pass(input, |ctx| {
                    action = ui::bubble(ctx, anchor(), &view, &mut draft, &mut picked, &str::to_string);
                });
                let button = output.shapes.iter().find_map(|shape| {
                    if let egui::epaint::Shape::Text(text) = &shape.shape
                        && text.galley.job.text == label
                    {
                        let rect = Rect::from_min_size(text.pos, text.galley.size());
                        if shape.clip_rect.intersect(screen).contains_rect(rect) {
                            return Some(rect.center());
                        }
                    }
                    None
                });
                assert!(output.platform_output.commands.is_empty(), "review text must not execute commands");
                output.drop_without_applying_deltas();
                (action, button)
            };
            for _ in 0..4 {
                frame(vec![]);
            }
            let mut button = None;
            for _ in 0..160 {
                // Native wheel input near the body's right edge also avoids the nested proposal text.
                let (_, visible) = frame(vec![
                    egui::Event::PointerMoved(pos2(390.0, 240.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, -240.0),
                        phase: egui::TouchPhase::Move,
                        source: egui::MouseWheelSource::Unknown,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
                if visible.is_some() {
                    button = visible;
                    break;
                }
            }
            let position = button.unwrap_or_else(|| panic!("{label} is outside the screen/clip after native wheel scrolling (failed={failed})"));
            let mut actual = None;
            for pressed in [true, false] {
                let (action, _) = frame(vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton { pos: position, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE },
                ]);
                actual = action.or(actual);
            }
            assert_eq!(actual, Some(expected), "{label} must remain clickable after scrolling (failed={failed})");
        }
    }
}
