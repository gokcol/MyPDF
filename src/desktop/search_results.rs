//! Search snippets use the same Unicode/whole-word matching as the PDF index.
use super::*;
use std::ops::Range;

#[derive(Default)]
struct Matches;
impl<'a> egui::cache::ComputerMut<(&'a str, &'a str, bool, bool), Vec<Range<usize>>> for Matches {
    fn compute(
        &mut self,
        (snippet, query, case_sensitive, whole_word): (&'a str, &'a str, bool, bool),
    ) -> Vec<Range<usize>> {
        PageText {
            text: snippet.into(),
            ..Default::default()
        }
        .find_with(
            query,
            0,
            viewer::SearchOptions {
                case_sensitive,
                whole_word,
            },
        )
        .into_iter()
        .map(|hit| hit.range)
        .collect()
    }
}

pub(super) fn row(
    ui: &mut egui::Ui,
    hit: &SearchHit,
    query: &str,
    options: viewer::SearchOptions,
    selected: bool,
) -> egui::Response {
    // Avoid repeating Unicode matching on idle redraws of large result lists.
    let ranges = ui.ctx().memory_mut(|memory| {
        memory
            .caches
            .cache::<egui::cache::FrameCache<Vec<Range<usize>>, Matches>>()
            .get((
                hit.snippet.as_str(),
                query,
                options.case_sensitive,
                options.whole_word,
            ))
    });
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &format!("Page {}\n", hit.page + 1),
        0.,
        egui::TextFormat {
            font_id: ui::strong(ui, 11.5),
            color: theme::accent(),
            ..Default::default()
        },
    );
    let plain = egui::TextFormat {
        font_id: FontId::proportional(13.),
        color: theme::ink(),
        ..Default::default()
    };
    let marked = egui::TextFormat {
        font_id: ui::strong(ui, 13.),
        background: theme::accent_soft(),
        underline: Stroke::new(1., theme::accent()),
        ..plain.clone()
    };
    let mut cursor = 0;
    for range in ranges {
        if range.start < cursor {
            continue;
        }
        job.append(&hit.snippet[cursor..range.start], 0., plain.clone());
        job.append(&hit.snippet[range.clone()], 0., marked.clone());
        cursor = range.end;
    }
    job.append(&hit.snippet[cursor..], 0., plain);
    job.wrap.max_width = (ui.available_width() - 16.).max(1.);
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    let (rect, response) = ui.allocate_exact_size(
        vec2(
            ui.available_width(),
            (galley.size().y + 16.).max(ui::target_size(32.)),
        ),
        egui::Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            ui.is_enabled(),
            selected,
            format!("Page {}: {}", hit.page + 1, hit.snippet),
        )
    });
    let fill = if selected {
        theme::accent_soft()
    } else if response.hovered() {
        theme::hover()
    } else {
        Color32::TRANSPARENT
    };
    let stroke = if response.has_focus() {
        Stroke::new(2., theme::accent())
    } else if selected {
        Stroke::new(1., theme::accent_line())
    } else {
        Stroke::NONE
    };
    ui.painter()
        .rect(rect, 8., fill, stroke, egui::StrokeKind::Inside);
    ui.painter()
        .galley(rect.min + vec2(8., 8.), galley, theme::ink());
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::cache::ComputerMut;

    #[test]
    fn snippet_matches_follow_unicode_case_whitespace_and_whole_word_options() {
        let snippet = "İstanbul audit auditee AUDIT\n report";
        let mut matcher = Matches;
        let words =
            |ranges: Vec<Range<usize>>| ranges.into_iter().map(|r| &snippet[r]).collect::<Vec<_>>();
        assert_eq!(
            words(matcher.compute((snippet, "audit", false, true))),
            vec!["audit", "AUDIT"]
        );
        assert_eq!(
            words(matcher.compute((snippet, "audit", true, true))),
            vec!["audit"]
        );
        assert_eq!(
            words(matcher.compute((snippet, "i\u{307}stanbul", false, true))),
            vec!["İstanbul"]
        );
        assert_eq!(
            words(matcher.compute((snippet, "audit report", false, false))),
            vec!["AUDIT\n report"]
        );
        assert!(matcher.compute((snippet, "  ", false, false)).is_empty());
    }

    #[test]
    fn whole_word_snippets_do_not_turn_cut_context_into_extra_matches() {
        let cases = [
            (format!("bobcat {} CAT", "x".repeat(19)), "cat", "CAT"),
            (format!("CAT {} catapult", "x".repeat(45)), "cat", "CAT"),
            (format!("bob.cat {} .CAT", "x".repeat(18)), ".cat", ".CAT"),
            (
                format!("CAT. {} cat.erpillar", "x".repeat(44)),
                "cat.",
                "CAT.",
            ),
            (
                format!("préİstanbul {} İSTANBUL", "x".repeat(14)),
                "i\u{307}stanbul",
                "İSTANBUL",
            ),
            (
                format!("CAFE\u{301} {} cafe\u{301}teria", "x".repeat(43)),
                "cafe\u{301}",
                "CAFE\u{301}",
            ),
            (
                format!("preaudit report {} AUDIT\n \treport", "x".repeat(10)),
                "audit report",
                "AUDIT\n \treport",
            ),
            (
                format!("AUDIT\n \treport {} audit reportage", "x".repeat(36)),
                "audit report",
                "AUDIT\n \treport",
            ),
        ];
        let mut matcher = Matches;
        for (original, query, actual) in cases {
            let expected_start = original.find(actual).unwrap();
            let page = PageText {
                text: original,
                ..Default::default()
            };
            let hits = page.find_with(
                query,
                7,
                viewer::SearchOptions {
                    case_sensitive: false,
                    whole_word: true,
                },
            );
            assert_eq!(hits.len(), 1, "Only the original whole-word match is a hit");
            assert_eq!(hits[0].page, 7);
            assert_eq!(
                hits[0].range,
                expected_start..expected_start + actual.len(),
                "Snippet trimming must not alter navigation offsets"
            );
            assert_eq!(&page.text[hits[0].range.clone()], actual);
            let ranges = matcher.compute((&hits[0].snippet, query, false, true));
            assert_eq!(
                ranges.len(),
                1,
                "Cut context must not create extra highlights: {:?}",
                hits[0].snippet
            );
            assert_eq!(
                hits[0].snippet[ranges[0].clone()].to_lowercase(),
                actual
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_lowercase(),
                "The full genuine match survives Unicode and whitespace normalization"
            );
        }
    }

    #[test]
    fn result_heading_and_padding_are_part_of_the_click_target() {
        let ctx = egui::Context::default();
        let hit = SearchHit {
            page: 2,
            range: 0..5,
            rects: vec![],
            snippet: "Audit findings and follow-up".into(),
        };
        let mut rect = Rect::NOTHING;
        let mut clicked = false;
        let mut frame = |events| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(240., 300.))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let response = row(ui, &hit, "Audit", Default::default(), false);
                        rect = response.rect;
                        clicked = response.clicked();
                    });
                },
            );
            (rect, clicked)
        };
        let (rect, _) = frame(vec![]);
        for point in [rect.min + vec2(12., 12.), rect.max - vec2(3., 3.)] {
            for pressed in [true, false] {
                let (_, click) = frame(vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
                if !pressed {
                    assert!(click, "entire row including heading/padding must activate");
                }
            }
        }
    }
}
