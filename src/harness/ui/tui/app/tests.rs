//! Unit tests for the TUI app (input editor + code-block helpers).

use super::*;
use crate::harness::event::{HarnessEvent, ToolStatus};

mod input_tests {
    use super::*;
    use crate::harness::ui::tui::input::{visual_row_col, wrap_visual};

    #[test]
    fn test_visual_up_down_move_between_rows() {
        let mut app = App::inline_for_tests("one two three\nfour five");
        app.input_inner_width = 5;
        let rows = wrap_visual(&app.input, 5);
        assert!(rows.len() > 2);
        let last_row = rows.len() - 1;
        app.input_cursor = app.input.chars().count();
        app.cursor_visual_up();
        let (r, _) = visual_row_col(&rows, app.input_cursor);
        assert!(r < last_row, "expected to move up, row={}", r);
        app.cursor_visual_down();
        let (r2, _) = visual_row_col(&rows, app.input_cursor);
        assert_eq!(r2, last_row);
    }

    #[test]
    fn test_line_boundaries_editing_keys() {
        let mut app = App::inline_for_tests("first line\nsecond line");
        app.input_cursor = 14;
        app.cursor_line_start();
        assert_eq!(app.input_cursor, 11);

        app.cursor_line_end();
        assert_eq!(app.input_cursor, 22);

        app.kill_to_line_start();
        assert_eq!(app.input, "first line\n");

        app.input_cursor = 10;
        app.kill_word_back();
        assert_eq!(app.input, "first \n");
        assert_eq!(app.input_cursor, 6);
    }

    #[test]
    fn test_clear_prompt_input_resets_editor() {
        let mut app = App::inline_for_tests("draft prompt");
        app.input_cursor = 5;
        app.history = vec!["old".into()];
        app.history_pos = Some(0);
        app.refresh_autocomplete();

        app.clear_prompt_input();
        assert!(app.input.is_empty());
        assert_eq!(app.input_cursor, 0);
        assert!(app.history_pos.is_none());
        assert!(app.autocomplete.is_none());
    }

    #[test]
    fn test_delete_forward() {
        let mut app = App::inline_for_tests("abc");
        app.input_cursor = 1;
        app.delete_forward();
        assert_eq!(app.input, "ac");
        app.input_cursor = 2;
        app.delete_forward();
        assert_eq!(app.input, "ac");
    }

    #[test]
    fn test_history_only_when_single_line() {
        let mut app = App::inline_for_tests("");
        app.history = vec!["old prompt".to_string()];
        assert!(!app.input.contains('\n'));
        app.history_up();
        assert_eq!(app.input, "old prompt");

        let mut app = App::inline_for_tests("linha1\nlinha2");
        app.history = vec!["old prompt".to_string()];
        let before = app.input.clone();
        app.cursor_visual_up();
        assert_eq!(app.history_pos, None);
        assert_eq!(app.input, before);
    }

    #[test]
    fn test_rebuild_transcript_from_session() {
        use crate::harness::session::{Message, Part, Role, ToolPart, ToolStatus};

        let mut app = App::inline_for_tests("");
        // Seed a stale line that should be wiped by the rebuild.
        app.push(LineKind::System, "stale line".to_string());

        // user → assistant (text + tool) → user → assistant.
        app.session.push_message(Message::user("first prompt"));
        app.session.push_message(Message::new(
            Role::Assistant,
            vec![
                Part::text("first reply"),
                Part::Tool(ToolPart {
                    id: "t1".into(),
                    name: "read".into(),
                    input: serde_json::json!({"path": "x"}),
                    status: ToolStatus::Completed,
                    output: "content".into(),
                    title: "read x".into(),
                    error: None,
                }),
            ],
        ));
        app.session.push_message(Message::user("second prompt"));
        app.session.push_message(Message::new(
            Role::Assistant,
            vec![Part::text("second reply")],
        ));

        app.rebuild_transcript_from_session();

        let kinds: Vec<LineKind> = app.lines.iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            vec![
                LineKind::User,
                LineKind::Assistant,
                LineKind::ToolOk,
                LineKind::User,
                LineKind::Assistant,
            ],
            "expected rebuilt kinds, got {:?}",
            kinds
        );
        // Stale line removed.
        assert!(!app.lines.iter().any(|l| l.text.contains("stale line")));
        // Tool line carries the title.
        assert!(app.lines.iter().any(|l| l.text.contains("read x")));
    }

    #[test]
    fn test_subagent_events_routed_to_panel_not_transcript() {
        let mut app = App::inline_for_tests("");
        let _before = app.lines.len();

        // Parent: a `task` tool starts → opens a panel.
        app.apply_event(HarnessEvent::ToolStart {
            session_id: "parent".into(),
            message_id: "m".into(),
            tool_id: "t1".into(),
            name: "task".into(),
            input: serde_json::json!({"agent": "explore", "prompt": "p"}),
            parent_session_id: None,
        });
        assert_eq!(app.subagent_panels.len(), 1);
        assert_eq!(app.subagent_panels[0].1.agent, "explore");

        // Child events (tagged with parent_session_id) go to the panel.
        app.apply_event(HarnessEvent::ToolStart {
            session_id: "child-1".into(),
            message_id: "cm".into(),
            tool_id: "ct1".into(),
            name: "grep".into(),
            input: serde_json::json!({}),
            parent_session_id: Some("parent".into()),
        });
        app.apply_event(HarnessEvent::ToolEnd {
            session_id: "child-1".into(),
            message_id: "cm".into(),
            tool_id: "ct1".into(),
            name: "grep".into(),
            status: ToolStatus::Completed,
            title: "grep: 3 matches".into(),
            output_preview: String::new(),
            diff: None,
            parent_session_id: Some("parent".into()),
        });
        // Child text deltas must NOT stream into the transcript.
        app.apply_event(HarnessEvent::TextDelta {
            session_id: "child-1".into(),
            message_id: "cm".into(),
            delta: "child text".into(),
            parent_session_id: Some("parent".into()),
        });

        let panel = &app.subagent_panels[0].1;
        assert_eq!(panel.child_session_id, "child-1");
        assert_eq!(panel.done, 1);
        assert!(panel.lines.iter().any(|l| l.contains("grep: 3 matches")));
        // Transcript untouched by child events.
        assert!(
            !app.lines.iter().any(|l| l.text.contains("child text")),
            "child events leaked into transcript"
        );
        assert!(app.streaming.is_none());

        // Parent ToolEnd for `task` finalizes the panel with the summary.
        app.apply_event(HarnessEvent::ToolEnd {
            session_id: "parent".into(),
            message_id: "m".into(),
            tool_id: "t1".into(),
            name: "task".into(),
            status: ToolStatus::Completed,
            title: "task (explore)".into(),
            output_preview: "Subagent `explore` result:\nfound 3 files".into(),
            diff: None,
            parent_session_id: None,
        });
        let panel = &app.subagent_panels[0].1;
        assert!(panel.finished);
        assert!(panel.summary.as_deref().unwrap().contains("found 3 files"));
        let label = App::subagent_panel_label(panel);
        assert!(label.starts_with("✓ explore"));
        assert!(label.contains("found 3 files"));
    }

    #[test]
    fn test_subagent_panel_label_running() {
        let panel = SubagentPanel {
            child_session_id: "c".into(),
            agent: "explore".into(),
            lines: vec!["· grep".into(), "✓ grep: 2".into()],
            done: 1,
            failed: 0,
            finished: false,
            summary: None,
        };
        let label = App::subagent_panel_label(&panel);
        assert_eq!(label, "⏳ explore — 1 tools");
    }
}

mod code_block_tests {
    use super::*;

    fn app_with_lines(lines: Vec<(&str, &str)>) -> App {
        let mut app = App::inline_for_tests("");
        app.lines = lines
            .into_iter()
            .map(|(k, t)| TranscriptLine {
                kind: match k {
                    "assistant" => LineKind::Assistant,
                    "user" => LineKind::User,
                    _ => LineKind::System,
                },
                text: t.to_string(),
            })
            .collect();
        app
    }

    #[test]
    fn test_last_code_block_extracts_fenced() {
        let app = app_with_lines(vec![
            ("user", "write a function"),
            ("assistant", "here:\n```rust\nfn main() {}\n```\ndone"),
        ]);
        assert_eq!(app.last_code_block().unwrap(), "fn main() {}");
    }

    #[test]
    fn test_last_code_block_takes_latest() {
        let app = app_with_lines(vec![
            ("assistant", "```\nfirst\n```"),
            ("assistant", "```\nsecond\n```"),
        ]);
        assert_eq!(app.last_code_block().unwrap(), "second");
    }

    #[test]
    fn test_last_code_block_none_without_fence() {
        let app = app_with_lines(vec![("assistant", "no code here")]);
        assert!(app.last_code_block().is_none());
    }

    #[test]
    fn test_last_code_block_multiline() {
        let app = app_with_lines(vec![(
            "assistant",
            "text\n```python\nprint(1)\nprint(2)\n```\nmore",
        )]);
        assert_eq!(app.last_code_block().unwrap(), "print(1)\nprint(2)");
    }
}

mod search_tests {
    use super::*;
    use crate::harness::ui::tui::app::search_lines;
    use crate::harness::ui::tui::app::SearchState;

    #[test]
    fn test_search_lines_case_insensitive_substring() {
        let lines = vec![
            TranscriptLine {
                kind: LineKind::User,
                text: "Fix the Parser bug".into(),
            },
            TranscriptLine {
                kind: LineKind::Assistant,
                text: "done, parser updated".into(),
            },
            TranscriptLine {
                kind: LineKind::System,
                text: "unrelated".into(),
            },
        ];
        assert_eq!(search_lines(&lines, "parser"), vec![0, 1]);
        assert_eq!(search_lines(&lines, "PARSER"), vec![0, 1]);
        assert_eq!(search_lines(&lines, "unrel"), vec![2]);
        assert!(search_lines(&lines, "missing").is_empty());
    }

    #[test]
    fn test_search_lines_empty_query_returns_empty() {
        let lines = vec![TranscriptLine {
            kind: LineKind::User,
            text: "hello".into(),
        }];
        assert!(search_lines(&lines, "").is_empty());
        assert!(search_lines(&lines, "   ").is_empty());
    }

    #[test]
    fn test_search_state_refresh_and_navigation() {
        let lines = vec![
            TranscriptLine {
                kind: LineKind::User,
                text: "alpha".into(),
            },
            TranscriptLine {
                kind: LineKind::Assistant,
                text: "beta".into(),
            },
            TranscriptLine {
                kind: LineKind::User,
                text: "ALPHA again".into(),
            },
        ];
        let mut st = SearchState::new();
        st.push_char('a');
        st.push_char('l');
        st.refresh(&lines);
        assert_eq!(st.matches, vec![0, 2]);
        st.move_sel(1);
        assert_eq!(st.current(), Some(2));
        st.move_sel(1);
        assert_eq!(st.current(), Some(0)); // wraps
        st.backspace();
        st.backspace();
        st.refresh(&lines);
        assert!(st.matches.is_empty());
        assert_eq!(st.current(), None);
    }

    #[test]
    fn test_search_state_no_matches_keeps_selected_zero() {
        let lines = vec![TranscriptLine {
            kind: LineKind::User,
            text: "x".into(),
        }];
        let mut st = SearchState::new();
        st.push_char('z');
        st.refresh(&lines);
        assert!(st.matches.is_empty());
        assert_eq!(st.selected, 0);
    }
}

mod thinking_tests {
    use super::*;
    use crate::harness::ui::tui::transcript::{collapse_thinking, THINKING_COLLAPSED_MARKER};

    fn reasoning(text: &str) -> TranscriptLine {
        TranscriptLine {
            kind: LineKind::Reasoning,
            text: text.to_string(),
        }
    }

    fn text(text: &str) -> TranscriptLine {
        TranscriptLine {
            kind: LineKind::Assistant,
            text: text.to_string(),
        }
    }

    #[test]
    fn test_collapse_thinking_collapsed_shows_one_line_with_counter() {
        let lines = vec![
            text("answer"),
            reasoning("first thought"),
            reasoning("second thought"),
            text("done"),
        ];
        let out = collapse_thinking(&lines, false);
        assert_eq!(out.len(), 3);
        let summary = &out[1];
        assert_eq!(summary.kind, LineKind::Reasoning);
        assert!(summary.text.contains("first thought"));
        assert!(summary.text.contains("27 chars thinking"));
        assert!(summary.text.contains(THINKING_COLLAPSED_MARKER));
    }

    #[test]
    fn test_collapse_thinking_expanded_keeps_all_lines() {
        let lines = vec![reasoning("a"), reasoning("b"), text("c")];
        let out = collapse_thinking(&lines, true);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].text, "a");
        assert_eq!(out[1].text, "b");
    }

    #[test]
    fn test_collapse_thinking_truncates_long_preview() {
        let long = "x".repeat(200);
        let lines = vec![reasoning(&long)];
        let out = collapse_thinking(&lines, false);
        assert_eq!(out.len(), 1);
        assert!(out[0].text.starts_with("xxxxxxxxxx"));
        assert!(out[0].text.contains("200 chars thinking"));
        assert!(out[0].text.contains('…'));
    }

    #[test]
    fn test_collapse_thinking_preserves_non_reasoning() {
        let lines = vec![text("a"), text("b")];
        let out = collapse_thinking(&lines, false);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].text, "a");
        assert_eq!(out[1].text, "b");
    }

    #[test]
    fn test_collapse_thinking_separate_runs() {
        let lines = vec![reasoning("one"), text("mid"), reasoning("two")];
        let out = collapse_thinking(&lines, false);
        assert_eq!(out.len(), 3);
        assert!(out[0].text.contains("3 chars thinking"));
        assert!(out[2].text.contains("3 chars thinking"));
    }
}

mod modal_queue_tests {
    use super::*;
    use crate::harness::tool::context::PermissionAskInput;
    use crate::harness::ui::tui::askers::{PermissionRequest, QuestionRequest};
    use tokio::sync::oneshot;

    fn perm_req() -> (PermissionRequest, oneshot::Receiver<bool>) {
        let (tx, rx) = oneshot::channel();
        let req = PermissionRequest {
            input: PermissionAskInput {
                tool: "bash".to_string(),
                args_summary: "ls".to_string(),
                path: None,
            },
            reply: tx,
        };
        (req, rx)
    }

    fn question_req() -> (QuestionRequest, oneshot::Receiver<Option<String>>) {
        let (tx, rx) = oneshot::channel();
        let req = QuestionRequest {
            question: "pick one".to_string(),
            options: vec!["a".to_string(), "b".to_string()],
            reply: tx,
        };
        (req, rx)
    }

    #[test]
    fn test_second_modal_queues_instead_of_overwriting() {
        let mut app = App::inline_for_tests("");
        let (p1, _rx1) = perm_req();
        let (p2, _rx2) = perm_req();

        app.enqueue_modal(Modal::Permission(p1));
        assert!(app.modal.is_some());
        assert!(app.modal_queue.is_empty());

        // Second ask while a modal is open → queued, not dropped.
        app.enqueue_modal(Modal::Permission(p2));
        assert!(app.modal.is_some());
        assert_eq!(app.modal_queue.len(), 1);

        // Closing the current modal opens the queued one.
        app.close_modal();
        assert!(app.modal.is_some());
        assert!(app.modal_queue.is_empty());
    }

    #[test]
    fn test_question_and_permission_are_both_served() {
        let mut app = App::inline_for_tests("");
        let (p, _prx) = perm_req();
        let (q, _qrx) = question_req();

        app.enqueue_modal(Modal::Permission(p));
        app.enqueue_modal(Modal::Question {
            req: q,
            draft: String::new(),
            cursor: 0,
        });
        assert_eq!(app.modal_queue.len(), 1);

        app.close_modal();
        assert!(matches!(app.modal, Some(Modal::Question { .. })));
        assert!(app.modal_queue.is_empty());
    }

    #[test]
    fn test_close_modal_with_empty_queue_clears() {
        let mut app = App::inline_for_tests("");
        let (p, _rx) = perm_req();
        app.enqueue_modal(Modal::Permission(p));
        app.close_modal();
        assert!(app.modal.is_none());
        assert!(app.modal_queue.is_empty());
    }
}
