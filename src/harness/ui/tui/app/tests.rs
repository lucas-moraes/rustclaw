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
        let before = app.lines.len();

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
        assert!(!app.streaming.is_some());

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
