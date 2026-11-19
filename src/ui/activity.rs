//! Compact, bounded tool activity disclosure, separate from answer/reasoning.
use eframe::egui;
use uuid::Uuid;

use crate::chat::{
    activity::{error_summary, single_line, ActivityKind, ActivityState},
    store::{ChatMessage, ChatRole},
};

use super::{find::find_ranges, theme, typography::ACTIVITY_SIZE};

/// The only activity text used by both Find and the widgets (including elided
/// labels' egui tooltips). Disclosure controls and totals aren't transcript
/// contents; a collapsed preview is a second view of a field, not a new hit.
pub struct Group {
    rows: Vec<Row>,
}

struct Row {
    state: Option<ActivityState>,
    kind: Option<ActivityKind>,
    owner: Option<Uuid>,
    detail: Option<String>,
    http_status: Option<u16>,
    calls: Vec<(Uuid, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Heading,
    Detail,
    Query(Uuid),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    row: Uuid,
    field: Field,
    occurrence: usize,
}

impl Row {
    fn id(&self) -> Uuid {
        self.calls[0].0
    }

    fn heading(&self) -> String {
        let icon = self.state.map_or("·", ActivityState::icon);
        let status = self.state.map_or("記錄", ActivityState::label);
        if self.calls.len() == 1 {
            format!("{icon} {} · {status}", self.calls[0].1)
        } else {
            // All call labels, including the first, appear exactly once in
            // details. Do not repeat the first query in the merged heading.
            format!("{icon} {status}（{} 次）", self.calls.len())
        }
    }

    fn details(&self) -> impl Iterator<Item = (Field, &str)> {
        self.detail
            .as_deref()
            .map(|s| (Field::Detail, s))
            .into_iter()
            .chain(
                self.calls
                    .iter()
                    .filter(|_| self.calls.len() > 1)
                    .map(|(id, label)| (Field::Query(*id), label.as_str())),
            )
    }
}

// v0.2.9 emitted a separate activity with this exact envelope after a tool
// returned ERROR:. Do not parse its arbitrary response body as a query, or
// mistake "Searching: Rust Vec<T>, 2 < 3, build failed: why?" for an error.
fn legacy_failure(text: &str) -> Option<String> {
    let warning = text
        .strip_prefix('⚠')?
        .trim_start_matches(['\u{fe0e}', '\u{fe0f}'])
        .strip_prefix(' ')?;
    let (name, error) = warning.split_once(" failed:")?;
    let kind = match name {
        "web_search" => ActivityKind::Search,
        "fetch_url" => ActivityKind::Fetch,
        _ => ActivityKind::Tool,
    };
    Some(error_summary(kind, error))
}

impl Group {
    pub fn new(messages: &[ChatMessage], active_owner: Option<Uuid>) -> Self {
        let mut rows: Vec<Row> = vec![];
        for message in messages.iter().filter(|m| m.role == ChatRole::Activity) {
            let http_status = message.activity.as_ref().and_then(|a| a.http_status);
            let (state, kind, owner, label, detail) = if let Some(activity) = &message.activity {
                let state = if active_owner.is_some() && activity.owner == active_owner {
                    activity.state
                } else {
                    activity.inactive_state()
                };
                (
                    Some(state),
                    Some(activity.kind),
                    activity.owner,
                    single_line(&activity.label, 180),
                    activity.detail.as_ref().map(|s| single_line(s, 500)),
                )
            } else if let Some(summary) = legacy_failure(&message.text) {
                (Some(ActivityState::Failed), None, None, summary, None)
            } else {
                (None, None, None, single_line(&message.text, 180), None)
            };
            if matches!(state, Some(ActivityState::Failed | ActivityState::Skipped))
                && detail.is_some()
            {
                if let Some(previous) = rows.iter_mut().find(|r| {
                    r.owner == owner
                        && r.state == state
                        && r.kind == kind
                        && (r.detail == detail
                            || (http_status.is_some() && r.http_status == http_status))
                        && (kind == Some(ActivityKind::Search) || r.calls[0].1 == label)
                }) {
                    previous.calls.push((message.id, label));
                    previous.detail = detail; // retain the latest pause notice
                    continue;
                }
            }
            rows.push(Row {
                state,
                kind,
                owner,
                detail,
                http_status,
                calls: vec![(message.id, label)],
            });
        }
        Self { rows }
    }

    pub fn find_hits(&self, query: &str) -> Vec<Hit> {
        let mut hits = Vec::new();
        for row in &self.rows {
            let heading = row.heading();
            for (field, text) in
                std::iter::once((Field::Heading, heading.as_str())).chain(row.details())
            {
                hits.extend((0..find_ranges(text, query).len()).map(|occurrence| Hit {
                    row: row.id(),
                    field,
                    occurrence,
                }));
            }
        }
        hits
    }

    fn summary(&self, active: bool) -> String {
        if self.rows.iter().all(|r| r.state.is_none()) {
            return format!("工具活動 · {} 筆記錄", self.rows.len());
        }
        let mut succeeded = 0;
        let mut failed = 0;
        let mut skipped = 0;
        let mut unknown = 0;
        let mut unsent = 0;
        let mut other = 0;
        for row in &self.rows {
            let count = row.calls.len();
            match row.state {
                Some(ActivityState::Succeeded) => succeeded += count,
                Some(ActivityState::Failed) => failed += count,
                Some(ActivityState::Skipped) => skipped += count,
                Some(ActivityState::OutcomeUnknown) => unknown += count,
                Some(ActivityState::CancelledBeforeSend | ActivityState::Denied) => unsent += count,
                Some(state) if state.is_pending() => {}
                _ => other += count,
            }
        }
        let progress = if active { "進行中 · " } else { "" };
        let mut summary = format!("工具活動 · {progress}完成 {succeeded}／失敗 {failed}");
        for (count, label) in [
            (skipped, "略過"),
            (unknown, "結果未知"),
            (unsent, "未執行"),
            (other, "其他"),
        ] {
            if count > 0 {
                summary.push_str(&format!("／{label} {count}"));
            }
        }
        summary
    }

    pub fn show(&self, ui: &mut egui::Ui, query: &str, hit: Option<Hit>, navigate: bool) {
        let Some(first) = self.rows.first() else {
            return;
        };
        let hit = hit.filter(|hit| self.rows.iter().any(|r| r.id() == hit.row));
        let navigate = navigate && hit.is_some();
        let pending = self
            .rows
            .iter()
            .rev()
            .find(|r| r.state.is_some_and(ActivityState::is_pending));
        let active = pending.is_some();
        let id = egui::Id::new(("tool-activity-group", first.id()));
        let mut disclosure = ui.data_mut(|d| d.get_temp::<Disclosure>(id).unwrap_or_default());
        if navigate {
            disclosure.expanded = true;
        } else if disclosure.active && !active && hit.is_none() {
            disclosure.expanded = false;
        }
        disclosure.active = active;
        ui.push_id(id, |ui| {
            ui.style_mut().override_font_id = Some(egui::FontId::proportional(ACTIVITY_SIZE));
            toggle(ui, &self.summary(active), &mut disclosure.expanded)
                .on_hover_text("展開／收合工具活動；不影響工具授權");
            if disclosure.expanded {
                let list = egui::ScrollArea::vertical()
                    .id_salt("activity-list")
                    .max_height(220.0)
                    .min_scrolled_height(80.0)
                    .auto_shrink([false, true])
                    .animated(false)
                    .show_viewport(ui, |ui, viewport| {
                        let mut target = None;
                        for row in &self.rows {
                            let row_hit = hit.filter(|h| h.row == row.id());
                            let rect = ui
                                .push_id(row.id(), |ui| row.show(ui, query, row_hit, navigate))
                                .inner;
                            if let Some(rect) = rect.filter(|_| navigate) {
                                target = Some(rect);
                            }
                        }
                        target.map(|rect| {
                            // The outer transcript may currently clip the *entire*
                            // group. Use this list's own viewport, not ui.clip_rect().
                            let center = ui.max_rect().top() + viewport.center().y;
                            ui.scroll_with_delta(egui::vec2(0.0, center - rect.center().y));
                            (rect, viewport.top())
                        })
                    });
                if let Some((rect, old_offset)) = list.inner {
                    // The inner scroll has consumed its request and clamped its
                    // actual offset. Reveal that same word in the OUTER transcript.
                    let rect = rect.translate(egui::vec2(0.0, old_offset - list.state.offset.y));
                    ui.scroll_to_rect(rect, Some(egui::Align::Center));
                }
            } else if let Some(row) = pending
                .or_else(|| {
                    // Leave the pause/no-request reason visible even if another
                    // backend has since failed. Stop outcomes also have counters.
                    self.rows
                        .iter()
                        .rev()
                        .find(|r| r.state == Some(ActivityState::Skipped))
                })
                .or_else(|| {
                    self.rows
                        .iter()
                        .rev()
                        .find(|r| r.state.is_some_and(|s| s != ActivityState::Succeeded))
                })
            {
                let heading = row.heading();
                let preview = match row.state {
                    Some(ActivityState::Failed | ActivityState::Skipped) => {
                        row.detail.as_deref().unwrap_or(&heading)
                    }
                    _ => &heading,
                };
                line(ui, preview, query, None, false);
            }
        });
        ui.ctx().data_mut(|data| data.insert_temp(id, disclosure));
        ui.add_space(4.0);
    }
}

#[derive(Clone, Copy, Default)]
struct Disclosure {
    expanded: bool,
    active: bool,
}

fn toggle(ui: &mut egui::Ui, text: &str, open: &mut bool) -> egui::Response {
    let arrow = if *open { "▾" } else { "▸" };
    let response = ui.add(
        egui::Label::new(
            egui::RichText::new(format!("{arrow} {text}"))
                .size(ACTIVITY_SIZE)
                .weak(),
        )
        .wrap()
        .selectable(false)
        .sense(egui::Sense::click()),
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, true, text));
    if response.clicked() {
        *open = !*open;
    }
    response
}

impl Row {
    fn show(
        &self,
        ui: &mut egui::Ui,
        query: &str,
        hit: Option<Hit>,
        navigate: bool,
    ) -> Option<egui::Rect> {
        let occurrence = |field| hit.filter(|h| h.field == field).map(|h| h.occurrence);
        let mut target = line(
            ui,
            &self.heading(),
            query,
            occurrence(Field::Heading),
            false,
        );
        if self.detail.is_some() || self.calls.len() > 1 {
            let id = ui.make_persistent_id("detail");
            let mut open = ui.data_mut(|d| d.get_temp::<bool>(id).unwrap_or(false));
            if navigate && hit.is_some_and(|h| h.field != Field::Heading) {
                open = true;
            }
            toggle(ui, "詳情", &mut open);
            ui.data_mut(|d| d.insert_temp(id, open));
            if open {
                ui.indent(id, |ui| {
                    for (field, text) in self.details() {
                        if let Some(rect) =
                            line(ui, text, query, occurrence(field), field == Field::Detail)
                        {
                            target = Some(rect);
                        }
                    }
                });
            }
        }
        target
    }
}

/// Even an offscreen match has a galley. Use its actual wrapped glyph row for
/// navigation; a long line can itself be taller than the bounded list.
fn line(
    ui: &mut egui::Ui,
    text: &str,
    query: &str,
    active: Option<usize>,
    wrap: bool,
) -> Option<egui::Rect> {
    let ranges = find_ranges(text, query);
    let font = egui::FontId::proportional(ACTIVITY_SIZE);
    if ranges.is_empty() {
        let label = egui::Label::new(egui::RichText::new(text).font(font).weak());
        ui.add(if wrap { label.wrap() } else { label.truncate() });
        return None;
    }
    let palette = theme::current_palette(ui.ctx());
    let normal = egui::TextFormat {
        font_id: font,
        color: ui.visuals().weak_text_color(),
        ..Default::default()
    };
    let mut job = egui::text::LayoutJob::default();
    let mut start = 0;
    for (i, range) in ranges.iter().enumerate() {
        job.append(&text[start..range.start], 0.0, normal.clone());
        let (background, color) = if Some(i) == active {
            (palette.accent, palette.accent_text)
        } else {
            (
                palette.accent.gamma_multiply(0.28),
                ui.visuals().text_color(),
            )
        };
        job.append(
            &text[range.clone()],
            0.0,
            egui::TextFormat {
                color,
                background,
                ..normal.clone()
            },
        );
        start = range.end;
    }
    job.append(&text[start..], 0.0, normal);
    if let Some(range) = active.and_then(|i| ranges.get(i)) {
        job.wrap.max_width = ui.available_width().max(1.0);
        let galley = ui.fonts(|f| f.layout_job(job));
        let mut cursor = egui::text::CCursor::new(text[..range.start].chars().count());
        cursor.prefer_next_row = true;
        let word_row = galley.pos_from_cursor(cursor);
        let response = ui.add(egui::Label::new(galley).halign(egui::Align::LEFT));
        Some(word_row.translate(response.rect.left_top().to_vec2()))
    } else {
        let label = egui::Label::new(job);
        ui.add(if wrap { label.wrap() } else { label.truncate() });
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::activity::ToolActivity;

    fn show(ui: &mut egui::Ui, messages: &[ChatMessage], owner: Option<Uuid>) {
        Group::new(messages, owner).show(ui, "", None, false);
    }

    fn failures() -> Vec<ChatMessage> {
        (0..30)
            .map(|i| {
                let mut activity =
                    ToolActivity::web("web_search", &format!(r#"{{"query":"中文 query {i}"}}"#));
                activity.fail("HTTP 202: <!DOCTYPE html>private-token");
                let mut message = ChatMessage::new(ChatRole::Activity, &activity.label);
                message.activity = Some(activity);
                message
            })
            .collect()
    }

    #[test]
    fn repeated_failures_merge_without_losing_queries_or_claiming_success() {
        let messages = failures();
        let group = Group::new(&messages, None);
        assert_eq!(group.rows.len(), 1);
        assert_eq!(group.rows[0].calls.len(), 30);
        assert_eq!(group.rows[0].state, Some(ActivityState::Failed));
        assert_eq!(group.find_hits("query").len(), 30);
        assert_eq!(group.find_hits("HTTP 202").len(), 1);
        let legacy = ChatMessage::new(
            ChatRole::Activity,
            "⚠︎ web_search failed: HTTP 202: <!DOCTYPE html>private-token",
        );
        let group = Group::new(&[legacy], None);
        assert_eq!(group.rows[0].calls[0].1, "搜尋未取得可用結果 · HTTP 202");
        assert_eq!(group.rows[0].state, Some(ActivityState::Failed));
    }

    #[test]
    fn pause_on_second_http_error_preserves_same_status_group_and_safe_find_hits() {
        let mut messages = vec![];
        let owner = Uuid::new_v4();
        for (i, paused) in [false, true].into_iter().enumerate() {
            let mut a = ToolActivity::new(ActivityKind::Search, format!("搜尋：query {i}"));
            a.owner = Some(owner);
            a.fail_http(reqwest::StatusCode::ACCEPTED, paused);
            let mut m = ChatMessage::new(ChatRole::Activity, &a.label);
            m.activity = Some(a);
            messages.push(m);
        }
        // The different text for the second attempt should not split the row.
        let group = Group::new(&messages, None);
        assert_eq!(group.rows.len(), 1);
        assert_eq!(group.rows[0].calls.len(), 2);
        assert!(group.summary(false).contains("失敗 2"));
        assert_eq!(group.find_hits("HTTP 202").len(), 1);
        assert_eq!(group.find_hits("本輪後續搜尋已暫停").len(), 1);
        assert_eq!(group.find_hits("query").len(), 2);
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
        frame_at(ctx, messages, owner, events, time, 300.0)
    }

    fn frame_at(
        ctx: &egui::Context,
        messages: &[ChatMessage],
        owner: Option<Uuid>,
        events: Vec<egui::Event>,
        time: f64,
        width: f32,
    ) -> (egui::FullOutput, egui::Rect) {
        let mut rect = egui::Rect::NOTHING;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 500.0),
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
    fn hovering_an_elided_legacy_warning_only_reveals_its_safe_summary() {
        let messages = [ChatMessage::new(
            ChatRole::Activity,
            "⚠︎ fetch_url failed: HTTP 429: <!DOCTYPE html><body>private-token</body>",
        )];
        let ctx = egui::Context::default();
        ctx.style_mut(|s| {
            s.interaction.tooltip_delay = 0.;
            s.interaction.show_tooltips_only_when_still = false;
        });
        let width = 120.;
        let (initial, _) = frame_at(&ctx, &messages, None, vec![], 0., width);
        let pos = text_rect(&initial, "HTTP 429").unwrap().0.center();
        frame_at(
            &ctx,
            &messages,
            None,
            vec![egui::Event::PointerMoved(pos)],
            1.,
            width,
        );
        let (hovered, _) = frame_at(&ctx, &messages, None, vec![], 2., width);
        let mut elided = false;
        let mut tooltip = false;
        for shape in &hovered.shapes {
            if let egui::Shape::Text(text) = &shape.shape {
                let displayed = text.galley.text();
                assert!(!displayed.contains("DOCTYPE") && !displayed.contains("private-token"));
                if displayed.contains("HTTP 429（請稍後再試）") {
                    if text.galley.elided {
                        elided = true
                    } else {
                        tooltip = true
                    }
                }
            }
        }
        assert!(
            elided && tooltip,
            "expected both the truncated preview and full tooltip"
        );
        assert!(messages[0].text.contains("DOCTYPE"));
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
            Group::new(&messages, None).rows[0].state,
            Some(ActivityState::Interrupted)
        );
        assert_eq!(
            Group::new(&messages, Some(owner)).rows[0].state,
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

    #[test]
    fn collapsed_stop_and_skipped_counts_use_effective_states() {
        let messages: Vec<_> = [
            (ActivityState::Succeeded, None),
            (ActivityState::Running, None), // reloaded after dispatch to an MCP tool
            (ActivityState::CancelledBeforeSend, None),
            (ActivityState::Skipped, Some("本輪搜尋已暫停，未再送出請求")),
        ]
        .into_iter()
        .map(|(state, detail)| {
            let mut activity = ToolActivity::new(ActivityKind::McpTool, "fixture");
            activity.state = state;
            activity.detail = detail.map(str::to_owned);
            let mut message = ChatMessage::new(ChatRole::Activity, "not a display field");
            message.activity = Some(activity);
            message
        })
        .collect();
        let ctx = egui::Context::default();
        let (output, rect) = frame(&ctx, &messages, None, vec![], 1.0);
        let summary = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text().contains("工具活動") => {
                    Some(text)
                }
                _ => None,
            })
            .unwrap();
        assert!(!summary.galley.elided);
        assert!(summary
            .galley
            .text()
            .contains("完成 1／失敗 0／略過 1／結果未知 1／未執行 1"));
        assert!(text_rect(&output, "未再送出請求").is_some());
        assert!(text_rect(&output, "詳情").is_none());
        assert!(rect.height() < 80.0);
    }
}
