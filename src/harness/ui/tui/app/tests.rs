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
            depth: 0,
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
            depth: 1,
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
            depth: 1,
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
            depth: 0,
        });
        let panel = &app.subagent_panels[0].1;
        assert!(panel.finished);
        assert!(panel.summary.as_deref().unwrap().contains("found 3 files"));
        let label = App::subagent_panel_label(panel);
        assert!(label.starts_with("└─ ✓L1 explore"), "got: {label}");
        assert!(label.contains("found 3 files"));
    }

    /// Regression: a subagent's `RunFinished` must NOT end the parent turn.
    /// Before the fix, child events arrived untagged, so the child's
    /// `RunFinished` was applied to the parent and flipped the UI to "idle"
    /// while the parent (and other subagents) were still running.
    #[test]
    fn test_child_run_finished_does_not_stop_parent() {
        let mut app = App::inline_for_tests("");
        // Parent turn starts.
        app.apply_event(HarnessEvent::RunStarted {
            session_id: "parent".into(),
            parent_session_id: None,
        });
        assert!(app.running);

        // A subagent finishes mid-turn (tagged with the parent id).
        app.apply_event(HarnessEvent::RunFinished {
            session_id: "child-1".into(),
            parent_session_id: Some("parent".into()),
        });
        assert!(
            app.running,
            "child RunFinished must not stop the parent turn"
        );

        // Only the parent's own RunFinished ends the turn.
        app.apply_event(HarnessEvent::RunFinished {
            session_id: "parent".into(),
            parent_session_id: None,
        });
        assert!(!app.running);
    }

    #[test]
    fn test_subagent_panel_label_running() {
        let panel = SubagentPanel {
            child_session_id: "c".into(),
            agent: "explore".into(),
            depth: 0,
            lines: vec!["· grep".into(), "✓ grep: 2".into()],
            done: 1,
            failed: 0,
            finished: false,
            summary: None,
        };
        let label = App::subagent_panel_label(&panel);
        assert_eq!(label, "⏳ explore — 1 tools");
    }

    #[test]
    fn test_subagent_panel_label_nested_shows_tree_and_level() {
        let panel = SubagentPanel {
            child_session_id: "c".into(),
            agent: "build".into(),
            depth: 2,
            lines: Vec::new(),
            done: 0,
            failed: 0,
            finished: false,
            summary: None,
        };
        let label = App::subagent_panel_label(&panel);
        // Depth 2 → one `│  ` segment then `└─ `, plus the `L2` level tag.
        assert_eq!(label, "│  └─ ⏳L2 build — 0 tools");
    }

    #[test]
    fn test_subagent_tree_prefix() {
        assert_eq!(App::subagent_tree_prefix(0), "");
        assert_eq!(App::subagent_tree_prefix(1), "└─ ");
        assert_eq!(App::subagent_tree_prefix(2), "│  └─ ");
        assert_eq!(App::subagent_tree_prefix(3), "│  │  └─ ");
    }

    #[test]
    fn test_nested_task_opens_panel_at_child_depth() {
        let mut app = App::inline_for_tests("");
        // A `task` call issued by a depth-1 subagent spawns a depth-2 panel.
        app.apply_event(HarnessEvent::ToolStart {
            session_id: "child-1".into(),
            message_id: "m".into(),
            tool_id: "t2".into(),
            name: "task".into(),
            input: serde_json::json!({"agent": "build", "prompt": "p"}),
            parent_session_id: Some("parent".into()),
            depth: 1,
        });
        assert_eq!(app.subagent_panels.len(), 1);
        assert_eq!(app.subagent_panels[0].1.depth, 2);
        assert!(!app.subagent_panels[0].1.finished);

        // The subagent's `task` ToolEnd finalizes the nested panel.
        app.apply_event(HarnessEvent::ToolEnd {
            session_id: "child-1".into(),
            message_id: "m".into(),
            tool_id: "t2".into(),
            name: "task".into(),
            status: ToolStatus::Completed,
            title: "task (build)".into(),
            output_preview: "nested result".into(),
            diff: None,
            parent_session_id: Some("parent".into()),
            depth: 1,
        });
        let nested = &app.subagent_panels[0].1;
        assert!(nested.finished);
        assert_eq!(nested.summary.as_deref(), Some("nested result"));
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

mod model_picker_tests {
    use super::*;

    #[test]
    fn test_open_model_picker_for_current_keeps_provider_and_preselects() {
        let mut app = App::inline_for_tests("");
        // RuntimeConfig::default() → provider "opencode-go", model "deepseek-v4-flash".
        let provider = app.runtime.config.provider.clone();
        let model = app.runtime.config.model.clone();
        assert_eq!(provider, "opencode-go");
        assert_eq!(model, "deepseek-v4-flash");

        app.open_model_picker_for_current();

        let picker = app.model_picker.as_ref().expect("picker should open");
        // Directly at the model stage, keeping the current provider.
        assert!(picker.stage_models, "should open at the model stage");
        assert_eq!(picker.provider, provider);
        // The active model is pre-selected in the provider's model list.
        let items = picker.items();
        let expected = items
            .iter()
            .position(|m| m == &model)
            .expect("active model should be in the list");
        assert_eq!(picker.selected, expected);
    }

    #[test]
    fn test_open_model_picker_for_current_busy_does_not_open() {
        let mut app = App::inline_for_tests("");
        app.running = true;
        app.open_model_picker_for_current();
        assert!(app.model_picker.is_none());
    }
}

mod mouse_scroll_tests {
    use super::*;

    #[test]
    fn test_mouse_scroll_returns_false_when_no_overlay() {
        let mut app = App::inline_for_tests("");
        app.scroll = 10;
        // No overlay open → mouse_scroll reports "not consumed" (returns false)
        // so the caller scrolls the transcript. It must not touch the transcript.
        assert!(!app.mouse_scroll(3));
        assert_eq!(app.scroll, 10);
    }

    #[test]
    fn test_mouse_scroll_rolls_model_picker_not_transcript() {
        let mut app = App::inline_for_tests("");
        app.open_model_picker_for_current();
        let picker = app.model_picker.as_mut().unwrap();
        picker.scroll_offset = 0;
        app.scroll = 10;

        // Overlay open → mouse scroll is consumed by the picker.
        assert!(app.mouse_scroll(3));
        assert_eq!(app.model_picker.as_ref().unwrap().scroll_offset, 3);
        // The transcript scroll is left untouched.
        assert_eq!(app.scroll, 10);
    }

    #[test]
    fn test_mouse_scroll_rolls_skill_picker() {
        let mut app = App::inline_for_tests("");
        // Build a skill picker with several entries.
        let mut picker = SkillPickerState::open(&app).unwrap_or_else(|| {
            // Fallback: construct one directly with fake ids.
            SkillPickerState {
                selected: 0,
                checked: vec![false; 5],
                ids: (0..5).map(|i| format!("skill-{i}")).collect(),
                scroll_offset: 0,
            }
        });
        if picker.ids.len() < 5 {
            picker.ids = (0..5).map(|i| format!("skill-{i}")).collect();
            picker.checked = vec![false; 5];
        }
        app.skill_picker = Some(picker);
        app.scroll = 10;

        assert!(app.mouse_scroll(2));
        assert_eq!(app.skill_picker.as_ref().unwrap().scroll_offset, 2);
        assert_eq!(app.scroll, 10);
    }
}

#[cfg(test)]
mod scroll_tests {
    use super::*;
    use crate::harness::ui::tui::app::keys::handle_key;

    #[test]
    fn test_scroll_by_clamps_at_zero() {
        let mut app = App::inline_for_tests("x");
        app.scroll = 0;
        app.scroll_by(-5);
        assert_eq!(app.scroll, 0);
        assert!(!app.stick_bottom);
    }

    #[test]
    fn test_clamp_scroll_stick_bottom_follows_end() {
        let mut app = App::inline_for_tests("x");
        // stick_bottom: scroll pinned to max
        app.stick_bottom = true;
        app.clamp_scroll(100, 10);
        assert_eq!(app.scroll, 90);
        assert!(app.stick_bottom);

        // scrolled up: clamped but not stuck
        app.stick_bottom = false;
        app.scroll = 500;
        app.clamp_scroll(100, 10);
        assert_eq!(app.scroll, 90);
        assert!(app.stick_bottom, "reaching the end re-engages stick_bottom");

        // mid-scroll: stays put
        app.stick_bottom = false;
        app.scroll = 40;
        app.clamp_scroll(100, 10);
        assert_eq!(app.scroll, 40);
        assert!(!app.stick_bottom);
    }

    #[test]
    fn test_clamp_scroll_total_smaller_than_view() {
        let mut app = App::inline_for_tests("x");
        app.stick_bottom = false;
        app.scroll = 7;
        app.clamp_scroll(5, 10);
        assert_eq!(app.scroll, 0);
        assert!(app.stick_bottom);
    }

    #[test]
    fn test_clear_transcript_resets_scroll_state() {
        let mut app = App::inline_for_tests("x");
        app.scroll = 42;
        app.stick_bottom = false;
        app.clear_transcript();
        assert_eq!(app.scroll, 0);
        assert!(app.stick_bottom);
        assert!(app.streaming.is_none());
        assert!(app.tool_status.is_none());
    }

    #[tokio::test]
    async fn test_settings_modal_down_moves_through_all_rows() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = App::inline_for_tests("x");
        app.modal = Some(Modal::Settings { selected: 0 });
        let mut prompt_task = None;

        let rows = crate::harness::ui::tui::draw::modal::settings_rows(&app.runtime.config);
        assert!(
            rows.len() > 1,
            "settings modal must expose more than one row"
        );

        // Down must advance the highlight through every row (regression: the
        // handler used to navigate a 1-element list, so Down never moved).
        for expected in 1..rows.len() {
            handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                &mut prompt_task,
            )
            .await
            .unwrap();
            match app.modal {
                Some(Modal::Settings { selected }) => assert_eq!(selected, expected),
                _ => panic!("settings modal closed unexpectedly"),
            }
        }

        // Down at the last row stays put.
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            &mut prompt_task,
        )
        .await
        .unwrap();
        match app.modal {
            Some(Modal::Settings { selected }) => assert_eq!(selected, rows.len() - 1),
            _ => panic!("settings modal closed unexpectedly"),
        }

        // Up moves back.
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            &mut prompt_task,
        )
        .await
        .unwrap();
        match app.modal {
            Some(Modal::Settings { selected }) => assert_eq!(selected, rows.len() - 2),
            _ => panic!("settings modal closed unexpectedly"),
        }
    }

    /// The `/settings` modal no longer carries the Cursor knobs: those moved
    /// to the dedicated `/cursor` modal.
    #[test]
    fn test_settings_rows_exclude_cursor() {
        let app = App::inline_for_tests("x");
        let rows = crate::harness::ui::tui::draw::modal::settings_rows(&app.runtime.config);
        assert!(
            !rows
                .iter()
                .any(|(l, _, _)| l == "cursor_agent" || l == "cursor_model"),
            "cursor rows must live in the /cursor modal, not /settings"
        );
        let crows = crate::harness::ui::tui::draw::modal::cursor_rows(&app.runtime.config);
        let labels: Vec<&str> = crows.iter().map(|(l, _, _)| l.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "cursor_agent",
                "cursor_model",
                "cursor_plan",
                "cursor_plan_model"
            ]
        );
    }

    /// `/cursor` with no args opens the dedicated modal.
    #[tokio::test]
    async fn test_cursor_command_opens_modal() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = App::inline_for_tests("x");
        let mut prompt_task = None;
        app.input = "/cursor".to_string();
        app.input_cursor = app.input.chars().count();
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut prompt_task,
        )
        .await
        .unwrap();

        match app.modal {
            Some(Modal::Cursor { selected }) => assert_eq!(selected, 0),
            _ => panic!("expected the /cursor modal to open"),
        }
    }

    /// Space on `cursor_agent` flips the toggle and persists it.
    #[tokio::test]
    async fn test_cursor_modal_space_toggles_agent() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = App::inline_for_tests("x");
        let before = app.runtime.config.cursor_agent;
        app.modal = Some(Modal::Cursor { selected: 0 });
        let mut prompt_task = None;

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
            &mut prompt_task,
        )
        .await
        .unwrap();

        assert_ne!(app.runtime.config.cursor_agent, before);
        match app.modal {
            Some(Modal::Cursor { selected }) => assert_eq!(selected, 0),
            _ => panic!("cursor modal must stay open after toggling"),
        }
    }

    /// Enter on the `cursor_model` row must open the model picker, not close
    /// the modal (regression: the handler set `Modal::CursorModel` and then
    /// called `close_modal()`, which popped the queue and discarded it).
    #[tokio::test]
    async fn test_cursor_modal_enter_on_model_row_opens_picker() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = App::inline_for_tests("x");
        let rows = crate::harness::ui::tui::draw::modal::cursor_rows(&app.runtime.config);
        let model_row = rows
            .iter()
            .position(|(label, _, _)| label == "cursor_model")
            .expect("cursor modal must expose a cursor_model row");
        app.modal = Some(Modal::Cursor {
            selected: model_row,
        });
        let mut prompt_task = None;

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut prompt_task,
        )
        .await
        .unwrap();

        match app.modal {
            Some(Modal::CursorModel {
                selected,
                models,
                target,
            }) => {
                // "auto" is always first and pre-selected when unset.
                assert_eq!(models.first().map(|(id, _)| id.as_str()), Some("auto"));
                assert_eq!(selected, 0);
                assert_eq!(target, CursorModelTarget::Build);
            }
            other => panic!(
                "expected the cursor model picker, got {}",
                if other.is_some() {
                    "another modal"
                } else {
                    "None"
                }
            ),
        }
    }

    /// The `/cursor` modal exposes the plan knobs too, in order.
    #[test]
    fn test_cursor_rows_include_plan() {
        let app = App::inline_for_tests("x");
        let rows = crate::harness::ui::tui::draw::modal::cursor_rows(&app.runtime.config);
        let labels: Vec<&str> = rows.iter().map(|(l, _, _)| l.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "cursor_agent",
                "cursor_model",
                "cursor_plan",
                "cursor_plan_model",
            ]
        );
    }

    /// Space on the `cursor_plan` row flips the plan toggle independently.
    #[tokio::test]
    async fn test_cursor_modal_space_toggles_plan() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = App::inline_for_tests("x");
        let before = app.runtime.config.cursor_plan;
        let rows = crate::harness::ui::tui::draw::modal::cursor_rows(&app.runtime.config);
        let plan_row = rows
            .iter()
            .position(|(label, _, _)| label == "cursor_plan")
            .expect("cursor modal must expose a cursor_plan row");
        app.modal = Some(Modal::Cursor { selected: plan_row });
        let mut prompt_task = None;

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
            &mut prompt_task,
        )
        .await
        .unwrap();

        assert_ne!(app.runtime.config.cursor_plan, before);
        // The build toggle must be untouched by the plan row.
        assert!(!app.runtime.config.cursor_agent);
    }

    /// Enter on `cursor_plan_model` opens the picker tagged for the plan knob.
    #[tokio::test]
    async fn test_cursor_modal_plan_model_row_opens_plan_picker() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = App::inline_for_tests("x");
        let rows = crate::harness::ui::tui::draw::modal::cursor_rows(&app.runtime.config);
        let row = rows
            .iter()
            .position(|(label, _, _)| label == "cursor_plan_model")
            .expect("cursor modal must expose a cursor_plan_model row");
        app.modal = Some(Modal::Cursor { selected: row });
        let mut prompt_task = None;

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut prompt_task,
        )
        .await
        .unwrap();

        match app.modal {
            Some(Modal::CursorModel { target, .. }) => {
                assert_eq!(target, CursorModelTarget::Plan);
            }
            _ => panic!("expected the cursor plan model picker"),
        }
    }

    /// `/cursor plan on` flips only the plan toggle.
    #[tokio::test]
    async fn test_cursor_command_plan_on() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = App::inline_for_tests("x");
        let build_before = app.runtime.config.cursor_agent;
        app.input = "/cursor plan on".to_string();
        app.input_cursor = app.input.chars().count();
        let mut prompt_task = None;

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut prompt_task,
        )
        .await
        .unwrap();

        assert!(app.runtime.config.cursor_plan);
        assert_eq!(app.runtime.config.cursor_agent, build_before);
    }
}

fn text_delta(delta: &str) -> HarnessEvent {
    HarnessEvent::TextDelta {
        session_id: "s".into(),
        message_id: "m".into(),
        delta: delta.into(),
        parent_session_id: None,
    }
}

mod dirty_event_tests {
    use super::*;

    #[test]
    fn test_apply_event_empty_delta_does_not_mark_dirty() {
        let mut app = App::inline_for_tests("x");
        app.needs_redraw = false;
        app.apply_event(text_delta(""));
        assert!(!app.needs_redraw);
        assert!(app.pending_stream.is_empty());
        assert!(app.streaming.is_none());
    }

    #[test]
    fn test_apply_event_text_delta_marks_dirty() {
        let mut app = App::inline_for_tests("x");
        app.needs_redraw = false;
        app.apply_event(text_delta("hello"));
        assert!(app.needs_redraw);
        assert!(
            !app.pending_stream.is_empty() || app.streaming.is_some(),
            "delta should land in pending or streaming"
        );
    }

    #[test]
    fn test_streaming_does_not_set_stick_bottom() {
        let mut app = App::inline_for_tests("x");
        app.stick_bottom = false;
        app.scroll = 4;
        app.apply_event(text_delta("token"));
        assert!(!app.stick_bottom);
        assert_eq!(app.scroll, 4);
    }

    #[test]
    fn test_scroll_up_during_delta_keeps_offset() {
        let mut app = App::inline_for_tests("x");
        app.scroll = 20;
        app.scroll_by(-5);
        let kept = app.scroll;
        app.apply_event(text_delta("more"));
        assert_eq!(app.scroll, kept);
        assert!(!app.stick_bottom);
    }
}

mod editor_kill_tests {
    use super::*;
    use crate::harness::ui::tui::input::{visual_row_col, wrap_visual};

    #[test]
    fn test_cursor_word_left_right_stops_at_whitespace() {
        let mut app = App::inline_for_tests("foo bar baz");
        app.input_cursor = 11;
        app.cursor_word_left();
        assert_eq!(app.input_cursor, 8); // "baz"
        app.cursor_word_left();
        assert_eq!(app.input_cursor, 4); // "bar"
        app.cursor_word_right();
        assert_eq!(app.input_cursor, 8);
        let mut app = App::inline_for_tests("ab\ncd");
        app.input_cursor = 3; // start of second line
        app.cursor_word_left();
        assert_eq!(app.input_cursor, 3, "must not cross line start");
    }

    #[test]
    fn test_kill_to_line_end_ctrl_k() {
        let mut app = App::inline_for_tests("hello world");
        app.input_cursor = 6;
        app.kill_to_line_end();
        assert_eq!(app.input, "hello ");
        assert_eq!(app.kill_ring.as_deref(), Some("world"));
    }

    #[test]
    fn test_kill_to_line_start_fills_kill_ring() {
        let mut app = App::inline_for_tests("hello world");
        app.input_cursor = 6;
        app.kill_to_line_start();
        assert_eq!(app.input, "world");
        assert_eq!(app.kill_ring.as_deref(), Some("hello "));
    }

    #[test]
    fn test_yank_inserts_at_cursor() {
        let mut app = App::inline_for_tests("ab");
        app.input_cursor = 1;
        app.kill_ring = Some("XY".into());
        app.yank_kill_ring();
        assert_eq!(app.input, "aXYb");
        assert_eq!(app.input_cursor, 3);
    }

    #[test]
    fn test_alt_backspace_kills_word() {
        let mut app = App::inline_for_tests("one two");
        app.input_cursor = 7;
        app.kill_word_back();
        assert_eq!(app.input, "one ");
        assert_eq!(app.kill_ring.as_deref(), Some("two"));
    }

    #[test]
    fn test_wrap_visual_long_line_cursor_stays_in_box() {
        let long = "x".repeat(80);
        let mut app = App::inline_for_tests(&long);
        app.input_inner_width = 20;
        app.input_cursor = 80;
        let rows = wrap_visual(&app.input, 20);
        let (_, col) = visual_row_col(&rows, app.input_cursor);
        assert!(col <= 20, "cursor col {col} must stay in the input box");
    }
}

mod rebuild_tests {
    use super::*;
    use crate::harness::session::{Message, Role};
    use crate::harness::ui::tui::app::state::STREAM_FLUSH_CHARS;

    #[test]
    fn test_rebuild_from_session_preserves_line_kinds() {
        let mut app = App::inline_for_tests("");
        app.session.push_message(Message::user("u"));
        app.session.push_message(Message::new(
            Role::Assistant,
            vec![crate::harness::session::Part::text("a")],
        ));
        app.rebuild_transcript_from_session();
        let kinds: Vec<LineKind> = app.lines.iter().map(|l| l.kind).collect();
        assert_eq!(kinds, vec![LineKind::User, LineKind::Assistant]);
    }

    #[test]
    fn test_rebuild_after_compact_drops_old_user_assistant_pairs() {
        let mut app = App::inline_for_tests("");
        app.session.push_message(Message::user("old prompt"));
        app.session.push_message(Message::new(
            Role::Assistant,
            vec![crate::harness::session::Part::text("old reply")],
        ));
        app.rebuild_transcript_from_session();
        assert!(app.lines.iter().any(|l| l.text.contains("old prompt")));
        app.session.messages.clear();
        app.session
            .push_message(Message::user("[compacted summary]"));
        app.session.push_message(Message::new(
            Role::Assistant,
            vec![crate::harness::session::Part::text("recent")],
        ));
        app.rebuild_transcript_from_session();
        assert!(!app.lines.iter().any(|l| l.text.contains("old prompt")));
        assert!(app.lines.iter().any(|l| l.text.contains("recent")));
    }

    #[test]
    fn test_streaming_then_flush_becomes_assistant_line() {
        let mut app = App::inline_for_tests("");
        let chunk = "x".repeat(STREAM_FLUSH_CHARS);
        app.apply_event(text_delta(&chunk));
        app.flush_stream_now();
        assert!(app.streaming.is_none());
        assert!(app.pending_stream.is_empty());
        assert_eq!(app.lines.last().map(|l| l.kind), Some(LineKind::Assistant));
        assert!(app.lines.last().unwrap().text.contains('x'));
    }

    #[test]
    fn test_rebuild_preserves_scroll_when_not_stuck() {
        let mut app = App::inline_for_tests("");
        app.session.push_message(Message::user("keep"));
        app.stick_bottom = false;
        app.scroll = 3;
        app.rebuild_transcript_from_session();
        assert_eq!(app.scroll, 3);
        assert!(!app.stick_bottom);
    }
}

mod scroll_extra_tests {
    use super::*;
    use crate::harness::ui::tui::app::keys::handle_key;
    use crate::harness::ui::tui::app::state::ThemePickerState;
    use crate::harness::ui::tui::selection::{CellPos, TextSelection};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[tokio::test]
    async fn test_ctrl_home_end_transcript_when_input_empty() {
        let mut app = App::inline_for_tests("");
        app.scroll = 12;
        app.stick_bottom = false;
        let mut prompt_task = None;
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL),
            &mut prompt_task,
        )
        .await
        .unwrap();
        assert_eq!(app.scroll, 0);
        assert!(!app.stick_bottom);
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL),
            &mut prompt_task,
        )
        .await
        .unwrap();
        assert!(app.stick_bottom);
    }

    #[test]
    fn test_page_size_matches_viewport() {
        let mut app = App::inline_for_tests("x");
        app.last_view_h = 20;
        assert_eq!(app.page_scroll_delta(), 19);
        app.last_view_h = 1;
        assert_eq!(app.page_scroll_delta(), 1);
    }

    #[test]
    fn test_selection_survives_scroll_by() {
        let mut app = App::inline_for_tests("x");
        app.selection = Some(TextSelection::new(CellPos::new(4, 2)));
        let before = app.selection.clone();
        app.scroll_by(3);
        assert_eq!(app.selection, before);
    }

    #[test]
    fn test_offset_after_resize_clamps_without_jump() {
        let mut app = App::inline_for_tests("x");
        app.stick_bottom = false;
        app.scroll = 40;
        app.clamp_scroll(100, 24);
        assert_eq!(app.scroll, 40);
        app.clamp_scroll(50, 20);
        assert_eq!(app.scroll, 30); // max = 50-20
        assert!(!app.stick_bottom || app.scroll == 30);
    }

    #[test]
    fn test_theme_picker_consumes_mouse_scroll() {
        let mut app = App::inline_for_tests("x");
        // Don't use open_theme_picker — NO_COLOR in the env locks it.
        app.theme_picker = Some(ThemePickerState {
            selected: 0,
            scroll_offset: 0,
            original: "cyberclaw".into(),
        });
        app.scroll = 9;
        assert!(app.mouse_scroll(1));
        assert_eq!(app.theme_picker.as_ref().unwrap().scroll_offset, 1);
        assert_eq!(app.scroll, 9);
    }

    #[test]
    fn test_resize_clamps_scroll() {
        let mut app = App::inline_for_tests("x");
        app.stick_bottom = false;
        app.scroll = 80;
        app.clamp_scroll(40, 10);
        assert_eq!(app.scroll, 30);
    }

    #[test]
    fn test_resize_keeps_selection_anchor_line() {
        let mut app = App::inline_for_tests("x");
        app.selection = Some(TextSelection {
            anchor: CellPos::new(2, 0),
            head: CellPos::new(2, 4),
            dragging: false,
        });
        // Resize must not clear a committed selection (runner no longer does).
        assert!(app.selection.is_some());
        app.clamp_scroll(100, 20);
        assert_eq!(app.selection.as_ref().unwrap().anchor.row, 2);
    }
}

mod snapshot_and_picker_tests {
    use super::*;
    use crate::harness::ui::tui::app::keys::handle_key;
    use crate::harness::ui::tui::theme::Theme;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn prep_snapshot(app: &mut App) {
        app.splash = None;
        app.tick = 0;
        app.theme = Theme::mono();
        app.theme_id = Theme::index_of("mono");
        app.needs_redraw = true;
    }

    fn buffer_has(buf: &ratatui::buffer::Buffer, needle: &str) -> bool {
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s.contains(needle)
    }

    #[test]
    fn test_snapshot_idle_empty_has_prompt() {
        let mut app = App::inline_for_tests("");
        prep_snapshot(&mut app);
        let buf = crate::harness::ui::tui::draw::render_to_buffer(&mut app, 80, 24);
        assert!(
            buffer_has(&buf, "prompt") || buffer_has(&buf, "idle") || buffer_has(&buf, "help"),
            "idle frame should show chrome"
        );
    }

    #[test]
    fn test_snapshot_user_assistant_bubbles() {
        let mut app = App::inline_for_tests("");
        prep_snapshot(&mut app);
        app.push(LineKind::User, "hello from user");
        app.push(LineKind::Assistant, "hello from assistant");
        let buf = crate::harness::ui::tui::draw::render_to_buffer(&mut app, 100, 30);
        assert!(
            buffer_has(&buf, "hello") && buffer_has(&buf, "assistant"),
            "expected user/assistant text in the buffer"
        );
    }

    #[test]
    fn test_snapshot_status_bar_idle() {
        let mut app = App::inline_for_tests("");
        prep_snapshot(&mut app);
        let buf = crate::harness::ui::tui::draw::render_to_buffer(&mut app, 80, 24);
        assert!(buffer_has(&buf, "idle") || buffer_has(&buf, "ctx"));
    }

    #[test]
    fn test_snapshot_settings_modal() {
        let mut app = App::inline_for_tests("");
        prep_snapshot(&mut app);
        app.modal = Some(Modal::Settings { selected: 0 });
        let buf = crate::harness::ui::tui::draw::render_to_buffer(&mut app, 80, 24);
        assert!(
            buffer_has(&buf, "settings") || buffer_has(&buf, "iterations"),
            "settings modal title/rows"
        );
    }

    #[test]
    fn test_snapshot_skill_picker() {
        let mut app = App::inline_for_tests("");
        prep_snapshot(&mut app);
        app.skill_picker = Some(SkillPickerState {
            selected: 0,
            checked: vec![false, true],
            ids: vec!["alpha".into(), "beta".into()],
            scroll_offset: 0,
        });
        let buf = crate::harness::ui::tui::draw::render_to_buffer(&mut app, 80, 24);
        assert!(buffer_has(&buf, "alpha") || buffer_has(&buf, "session memory"));
    }

    #[tokio::test]
    async fn test_model_picker_open_cancel() {
        let mut app = App::inline_for_tests("");
        app.open_models_picker();
        assert!(app.model_picker.is_some());
        let mut prompt_task = None;
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &mut prompt_task,
        )
        .await
        .unwrap();
        assert!(app.model_picker.is_none());
    }

    #[tokio::test]
    async fn test_skill_picker_toggle_and_enter() {
        let mut app = App::inline_for_tests("");
        app.skill_picker = Some(SkillPickerState {
            selected: 0,
            checked: vec![false],
            ids: vec!["demo".into()],
            scroll_offset: 0,
        });
        let mut prompt_task = None;
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
            &mut prompt_task,
        )
        .await
        .unwrap();
        assert!(app.skill_picker.as_ref().unwrap().checked[0]);
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut prompt_task,
        )
        .await
        .unwrap();
        assert!(app.skill_picker.is_none());
    }

    #[test]
    fn test_resume_picker_selects_session_id() {
        use crate::harness::session::store::SessionSummary;
        let mut app = App::inline_for_tests("");
        app.resume_picker = Some(ResumePickerState {
            sessions: vec![SessionSummary {
                id: "sess-42".into(),
                agent: "build".into(),
                cwd: std::path::PathBuf::new(),
                created_at: String::new(),
                updated_at: String::new(),
                message_count: 0,
                preview: "hello".into(),
                title: Some("hello".into()),
                parent_id: None,
            }],
            selected: 0,
            rename_input: None,
            scroll_offset: 0,
        });
        let id = app.resume_picker.as_ref().unwrap().sessions[0].id.clone();
        assert_eq!(id, "sess-42");
    }

    #[test]
    fn test_readme_documents_rustclaw_ui_cli() {
        let readme = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md"));
        assert!(readme.contains("RUSTCLAW_UI=cli"));
        assert!(readme.contains("RUSTCLAWUI"));
    }
}
