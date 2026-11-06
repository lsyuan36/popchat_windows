//! Compact, bounded tool activity disclosure, separate from answer/reasoning.
use eframe::egui;

use crate::chat::{
    activity::{single_line, ActivityKind, ActivityState},
    store::{ChatMessage, ChatRole},
};

use super::typography::ACTIVITY_SIZE;

#[derive(Clone)]
struct Row {
    label: String,
    state: Option<ActivityState>,
    kind: Option<ActivityKind>,
    detail: Option<String>,
    queries: Vec<String>,
}

fn rows(messages: &[ChatMessage], active_owner: Option<uuid::Uuid>) -> Vec<Row> {
    let mut rows: Vec<Row> = vec![];
    for message in messages.iter().filter(|m| m.role == ChatRole::Activity) {
        let row = if let Some(activity) = &message.activity {
            let state = if activity.state.is_pending()
                && (activity.owner.is_none() || activity.owner != active_owner)
            {
                ActivityState::Interrupted
            } else {
                activity.state
            };
            Row {
                label: single_line(&activity.label, 180),
                state: Some(state),
                kind: Some(activity.kind),
                detail: activity.detail.as_ref().map(|s| single_line(s, 500)),
                queries: vec![single_line(&activity.label, 180)],
            }
        } else {
            // Old transcripts are display-only: do not rewrite or discard them.
            // In particular, never re-expose previously saved HTML error bodies.
            let text = &message.text;
            let failure = text.contains(" failed:");
            let label = if failure {
                let error = text.split_once(" failed:").map(|(_, e)| e).unwrap_or("");
                crate::chat::activity::error_summary(
                    if text.contains("web_search") {
                        ActivityKind::Search
                    } else {
                        ActivityKind::Tool
                    },
                    error,
                )
            } else if text.contains('<') {
                "工具活動記錄（原始內容已隱藏）".into()
            } else {
                single_line(text, 180)
            };
            Row {
                label,
                state: None,
                kind: None,
                detail: None,
                queries: vec![],
            }
        };
        // Merge repeated failures of the same backend/category, not successful
        // operations. Keep every query accessible inside the bounded disclosure.
        if row.state == Some(ActivityState::Failed) && row.detail.is_some() {
            if let Some(previous) = rows.iter_mut().find(|r| {
                r.state == row.state
                    && r.kind == row.kind
                    && r.detail == row.detail
                    && (row.kind == Some(ActivityKind::Search) || r.label == row.label)
            }) {
                previous.queries.extend(row.queries);
                continue;
            }
        }
        rows.push(row);
    }
    rows
}

#[derive(Clone, Copy, Default)]
struct Disclosure {
    expanded: bool,
    active: bool,
}

pub fn show(ui: &mut egui::Ui, messages: &[ChatMessage], active_owner: Option<uuid::Uuid>) {
    let Some(first) = messages.first() else {
        return;
    };
    let rows = rows(messages, active_owner);
    let active = rows
        .iter()
        .any(|r| r.state.is_some_and(ActivityState::is_pending));
    let succeeded = messages
        .iter()
        .filter(|m| {
            m.activity
                .as_ref()
                .is_some_and(|a| a.state == ActivityState::Succeeded)
        })
        .count();
    let failed = messages
        .iter()
        .filter(|m| {
            m.activity
                .as_ref()
                .is_some_and(|a| a.state == ActivityState::Failed)
        })
        .count();
    let id = egui::Id::new(("tool-activity-group", first.id));
    let mut disclosure = ui
        .ctx()
        .data_mut(|data| data.get_temp::<Disclosure>(id).unwrap_or_default());
    if disclosure.active && !active {
        disclosure.expanded = false;
    }
    disclosure.active = active;
    let summary = if active {
        format!("工具活動 · 進行中 · 完成 {succeeded}／失敗 {failed}")
    } else if messages.iter().all(|m| m.activity.is_none()) {
        format!("工具活動 · {} 筆記錄", messages.len())
    } else {
        let other = messages.len().saturating_sub(succeeded + failed);
        format!(
            "工具活動 · 完成 {succeeded}／失敗 {failed}{}",
            if other > 0 {
                format!("／其他 {other}")
            } else {
                String::new()
            }
        )
    };
    ui.push_id(id, |ui| {
        ui.style_mut().override_font_id = Some(egui::FontId::proportional(ACTIVITY_SIZE));
        let header = ui.add(
            egui::Label::new(
                egui::RichText::new(format!(
                    "{} {summary}",
                    if disclosure.expanded { "▾" } else { "▸" }
                ))
                .size(ACTIVITY_SIZE)
                .weak(),
            )
            .truncate()
            .sense(egui::Sense::click()),
        );
        header.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, true, &summary)
        });
        if header
            .on_hover_text("展開／收合工具活動；不影響工具授權")
            .clicked()
        {
            disclosure.expanded = !disclosure.expanded;
        }
        if disclosure.expanded {
            let height = ui.available_height().clamp(80.0, 220.0);
            egui::ScrollArea::vertical()
                .id_salt("activity-list")
                .max_height(height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (index, row) in rows.iter().enumerate() {
                        ui.push_id(index, |ui| {
                            let state = row.state;
                            let count = row.queries.len();
                            let suffix = if count > 1 {
                                format!("（{count} 次）")
                            } else {
                                String::new()
                            };
                            let text = format!(
                                "{} {} · {}{suffix}",
                                state.map_or("·", ActivityState::icon),
                                row.label,
                                state.map_or("記錄", ActivityState::label)
                            );
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(text).size(ACTIVITY_SIZE).weak(),
                                )
                                .truncate(),
                            );
                            if row.detail.is_some() || count > 1 {
                                egui::CollapsingHeader::new(
                                    egui::RichText::new("詳情").size(ACTIVITY_SIZE),
                                )
                                .id_salt("detail")
                                .show(ui, |ui| {
                                    if let Some(detail) = &row.detail {
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(detail)
                                                    .size(ACTIVITY_SIZE)
                                                    .weak(),
                                            )
                                            .wrap()
                                            .selectable(true),
                                        );
                                    }
                                    if count > 1 {
                                        for query in &row.queries {
                                            ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(query)
                                                        .size(ACTIVITY_SIZE)
                                                        .weak(),
                                                )
                                                .truncate(),
                                            );
                                        }
                                    }
                                });
                            }
                        });
                    }
                });
        } else if let Some(row) = rows
            .iter()
            .rev()
            .find(|r| r.state.is_some_and(ActivityState::is_pending))
        {
            let state = row.state.unwrap();
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!(
                        "{} {} · {}",
                        state.icon(),
                        row.label,
                        state.label()
                    ))
                    .size(ACTIVITY_SIZE)
                    .weak(),
                )
                .truncate(),
            );
        } else if let Some(row) = rows
            .iter()
            .rev()
            .find(|r| r.state == Some(ActivityState::Failed))
        {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!(
                        "⚠ {}",
                        row.detail.as_deref().unwrap_or("工具未完成")
                    ))
                    .size(ACTIVITY_SIZE)
                    .weak(),
                )
                .truncate(),
            );
        } else if let Some(row) = rows.iter().rev().find(|r| {
            matches!(
                r.state,
                Some(ActivityState::Denied | ActivityState::Cancelled | ActivityState::Interrupted)
            )
        }) {
            let state = row.state.unwrap();
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!(
                        "{} {} · {}",
                        state.icon(),
                        row.label,
                        state.label()
                    ))
                    .size(ACTIVITY_SIZE)
                    .weak(),
                )
                .truncate(),
            );
        }
    });
    ui.ctx().data_mut(|data| data.insert_temp(id, disclosure));
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::activity::ToolActivity;

    fn failures() -> Vec<ChatMessage> {
        (0..30)
            .map(|i| {
                let mut activity =
                    ToolActivity::web("web_search", &format!(r#"{{"query":"中文 query {i}"}}"#));
                activity.finish("ERROR: HTTP 202: <!DOCTYPE html>private-token");
                let mut message = ChatMessage::new(ChatRole::Activity, &activity.label);
                message.activity = Some(activity);
                message
            })
            .collect()
    }

    #[test]
    fn repeated_failures_merge_without_losing_queries_or_claiming_success() {
        let messages = failures();
        let rows = rows(&messages, None);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].queries.len(), 30);
        assert_eq!(rows[0].state, Some(ActivityState::Failed));
        let legacy = ChatMessage::new(
            ChatRole::Activity,
            "⚠︎ web_search failed: HTTP 202: <!DOCTYPE html>private-token",
        );
        let rows = super::rows(&[legacy], None);
        assert_eq!(rows[0].label, "搜尋未取得可用結果 · HTTP 202");
        assert_eq!(rows[0].state, None);
    }

    #[test]
    fn compact_and_expanded_activity_are_bounded_and_stay_small() {
        for dark in [true, false] {
            for width in [240.0, 420.0] {
                for expanded in [false, true] {
                    let ctx = egui::Context::default();
                    if dark {
                        ctx.set_visuals(egui::Visuals::dark());
                    } else {
                        ctx.set_visuals(egui::Visuals::light());
                    }
                    let messages = failures();
                    let id = egui::Id::new(("tool-activity-group", messages[0].id));
                    ctx.data_mut(|data| {
                        data.insert_temp(
                            id,
                            Disclosure {
                                expanded,
                                active: false,
                            },
                        )
                    });
                    let mut activity_rect = egui::Rect::NOTHING;
                    let output = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, 500.0),
                            )),
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                super::super::typography::apply(ui, 24);
                                activity_rect =
                                    ui.scope(|ui| show(ui, &messages, None)).response.rect;
                                ui.label("NORMAL_ANSWER_BODY");
                            });
                        },
                    );
                    assert!(activity_rect.width() <= width);
                    assert!(activity_rect.height() < if expanded { 280.0 } else { 80.0 });
                    for shape in &output.shapes {
                        if let egui::Shape::Text(text) = &shape.shape {
                            assert!(!text.galley.job.text.contains("DOCTYPE"));
                            assert!(!text.galley.job.text.contains("private-token"));
                            let size = if text.galley.job.text == "NORMAL_ANSWER_BODY" {
                                24.0
                            } else {
                                ACTIVITY_SIZE
                            };
                            assert!(text.galley.job.sections.iter().all(|section| section
                                .format
                                .font_id
                                .size
                                == size));
                        }
                    }
                }
            }
        }
    }

    fn frame(
        ctx: &egui::Context,
        messages: &[ChatMessage],
        owner: Option<uuid::Uuid>,
        events: Vec<egui::Event>,
        time: f64,
    ) -> (egui::FullOutput, egui::Rect) {
        let mut rect = egui::Rect::NOTHING;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(300.0, 500.0),
                )),
                events,
                time: Some(time / 60.0),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    rect = ui.scope(|ui| show(ui, messages, owner)).response.rect;
                    ui.label("ANSWER_UNCHANGED");
                });
            },
        );
        (output, rect)
    }

    fn text_rect(output: &egui::FullOutput, needle: &str) -> Option<(egui::Rect, egui::Rect)> {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text.contains(needle) => Some((
                egui::Rect::from_min_size(text.pos, text.galley.size()),
                shape.clip_rect,
            )),
            _ => None,
        })
    }

    fn pointer(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    #[test]
    fn disclosure_click_details_and_long_list_wheel_are_bounded_and_answer_stays_visible() {
        let ctx = egui::Context::default();
        ctx.style_mut(|s| s.animation_time = 0.0);
        let messages: Vec<_> = (0..80)
            .map(|i| {
                let mut activity = ToolActivity::new(
                    ActivityKind::McpTool,
                    format!("MCP fixture operation {i:03}"),
                );
                activity.state = ActivityState::Succeeded;
                activity.detail = Some(format!("safe-detail-{i:03}"));
                let mut message = ChatMessage::new(ChatRole::Activity, &activity.label);
                message.activity = Some(activity);
                message
            })
            .collect();
        let _ = frame(&ctx, &messages, None, vec![], 0.0);
        let (initial, rect) = frame(&ctx, &messages, None, vec![], 1.0);
        assert!(rect.height() < 60.0);
        let header =
            text_rect(&initial, "工具活動").unwrap().0.left_center() + egui::vec2(8.0, 0.0);
        let _ = frame(&ctx, &messages, None, pointer(header, true), 2.0);
        let (expanded, rect) = frame(&ctx, &messages, None, pointer(header, false), 3.0);
        assert!(
            rect.height() > 200.0 && rect.height() < 270.0,
            "expanded group rect={rect:?}, state={:?}",
            ctx.data_mut(|data| data
                .get_temp::<Disclosure>(egui::Id::new(("tool-activity-group", messages[0].id))))
                .map(|d| d.expanded)
        );
        assert!(text_rect(&expanded, "ANSWER_UNCHANGED").unwrap().0.bottom() < 500.0);
        let detail = text_rect(&expanded, "詳情").unwrap().0.center();
        let _ = frame(&ctx, &messages, None, pointer(detail, true), 4.0);
        let (detailed, rect) = frame(&ctx, &messages, None, pointer(detail, false), 5.0);
        assert!(text_rect(&detailed, "safe-detail-000").is_some());
        assert!(rect.height() < 270.0);
        let mut bottom = frame(
            &ctx,
            &messages,
            None,
            vec![
                egui::Event::PointerMoved(egui::pos2(200.0, 120.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -10000.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            6.0,
        )
        .0;
        for tick in 0..10 {
            bottom = frame(&ctx, &messages, None, vec![], 7.0 + f64::from(tick)).0;
        }
        let (last, clip) = text_rect(&bottom, "operation 079").unwrap();
        assert!(clip.intersect(last).height() >= last.height().min(12.0));
        assert!(
            text_rect(&bottom, "operation 000").is_none_or(|(first, clip)| !clip.intersects(first))
        );
        assert!(text_rect(&bottom, "ANSWER_UNCHANGED").unwrap().0.bottom() < 500.0);
        let mut restored = frame(
            &ctx,
            &messages,
            None,
            vec![
                egui::Event::PointerMoved(egui::pos2(200.0, 120.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 10000.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            18.0,
        )
        .0;
        for tick in 0..10 {
            restored = frame(&ctx, &messages, None, vec![], 19.0 + f64::from(tick)).0;
        }
        let (first, clip) = text_rect(&restored, "operation 000").unwrap();
        assert!(clip.intersect(first).height() >= first.height().min(12.0));
        let _ = frame(&ctx, &messages, None, pointer(header, true), 30.0);
        let (_, rect) = frame(&ctx, &messages, None, pointer(header, false), 31.0);
        assert!(rect.height() < 60.0);
    }

    #[test]
    fn live_activity_shows_approval_collapse_on_completion_and_restored_pending_is_not_running() {
        let owner = uuid::Uuid::new_v4();
        let mut activity = ToolActivity::new(ActivityKind::McpTool, "MCP fixture");
        activity.owner = Some(owner);
        activity.state = ActivityState::WaitingApproval;
        let mut messages = vec![ChatMessage::new(ChatRole::Activity, "MCP fixture")];
        messages[0].activity = Some(activity.clone());
        assert_eq!(
            rows(&messages, None)[0].state,
            Some(ActivityState::Interrupted)
        );
        assert_eq!(
            rows(&messages, Some(owner))[0].state,
            Some(ActivityState::WaitingApproval)
        );
        let ctx = egui::Context::default();
        let (waiting, _) = frame(&ctx, &messages, Some(owner), vec![], 1.0);
        assert!(text_rect(&waiting, "等待你確認").is_some());
        let id = egui::Id::new(("tool-activity-group", messages[0].id));
        ctx.data_mut(|data| {
            data.insert_temp(
                id,
                Disclosure {
                    expanded: true,
                    active: true,
                },
            )
        });
        activity.state = ActivityState::Succeeded;
        messages[0].activity = Some(activity);
        let (_, rect) = frame(&ctx, &messages, Some(owner), vec![], 2.0);
        assert!(rect.height() < 60.0);
        assert!(
            !ctx.data_mut(|data| data.get_temp::<Disclosure>(id).unwrap())
                .expanded
        );
    }
}
