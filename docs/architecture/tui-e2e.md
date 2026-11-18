# Terminal end-to-end suite

The native terminal UI's end-to-end scenarios run through tmux on a developer's
machine. They drive the production CLI and its App Server with real terminal
input, resizes and captured output, against a deterministic model fixture. CI
does not run them: it keeps component render and keyboard checks, protocol
contract tests, and line-mode CLI flows.

## Command

```sh
cargo build --locked -p dimcode
node scripts/tui-tmux/run.mjs
```

Options and useful forms:

```sh
node scripts/tui-tmux/run.mjs --list                       # areas and scenario names
node scripts/tui-tmux/run.mjs --area approvals             # one area
node scripts/tui-tmux/run.mjs queue_holds_steers_and_deletes
node scripts/tui-tmux/run.mjs --keep --binary target/debug/dimcode
node scripts/tui-tmux/run.mjs --artifacts /tmp/tui-failure # failed cases go here
```

Prerequisites:

| Requirement | Notes |
| --- | --- |
| `target/debug/dimcode` | Built from the checkout; `--binary` names another build |
| tmux | 3.2 or newer, on macOS or Linux |
| Node.js | The version in `.node-version`; the suite and its fixture are plain ES modules |
| No network | The model fixture listens on loopback; the suite sets `DIMCODE_UPDATE_CHECK=0` |

The suite is Unix-only because tmux is. Windows keeps its native boundary checks
(`cargo test --locked -p dim-cli --lib` in the `windows-cli` job) and the
component-level terminal tests that do not need tmux.

## What a case runs

Each case gets a disposable state of its own, under a short path in the system
temporary directory (`/tmp/dimtui-<run>/<case>`), because a unix socket name is
bounded:

| Piece | Detail |
| --- | --- |
| Product home | `--home <case>/home` with `rust/` at mode 0700; the product home `~/.dimcode` is never touched |
| User home | `HOME=<case>/user-home` and `TMPDIR=<case>/tmp`, so terminal helpers and tools write nothing outside the case |
| tmux server | One per case, on `<case>/tmux.sock`; `remain-on-exit` keeps the pane's last screen and its exit status |
| Daemon | The production App Server: the CLI's own `app-server` subcommand (`--server` is not used) |
| Model | A loopback OpenAI-compatible fixture; prompts map to fixed replies and scripted tool calls (`exec`, `ask_user`) |
| Environment | `TERM=screen-256color`, `LANG=en_US.UTF-8`, `SHELL=/bin/sh`, `locale: en` in `config.json` |

The launcher scenario runs `npm/bin/dimcode.mjs` with a copied platform package,
so the wrapper's own code path starts the same native binary.

Input is real terminal input: `tmux send-keys -l` for text, `send-keys` for keys
(`C-j`, `Escape`, `BSpace`, `Down`), `send-keys -H` for bracketed pastes, and
`tmux resize-window` for resizes. Assertions read the pane with
`capture-pane -p -J` (the visible screen) and `capture-pane -p -J -S -` (the
scrollback above it). Waits are for text the interface draws; a short settle
wait appears only where a key acts on a queued row that is still being rewritten.

## Determinism and teardown

The fixture model is the only non-production component. Every scenario scripts
its replies, so a prompt has one expected answer; the tool calls a scenario needs
(`exec`, `ask_user`) are scripted per request. Requests are recorded with their
messages, so a scenario can assert what reached the model.

Teardown always runs: the tmux server is killed (ending the pane's CLI), the
daemon is stopped through `dimcode --home … daemon stop` with a `pkill` fallback,
the fixture server is closed, and the case directory is removed. `--keep` leaves
the home and raw streams in place for inspection.

## Scenarios

| Area | Scenario | Verifies |
| --- | --- | --- |
| terminal-modes | `scrollback_streams_and_exits_cleanly` | The default scrollback mode, streamed answer, committed history, `/exit`, `Bye!`, exit 0, no alternate screen, cursor restored, OSC 7501 working/clear |
| terminal-modes | `fullscreen_switches_screens_and_restores_the_terminal` | Fullscreen takes the alternate screen; `/tui` leaves and re-enters it; every keyboard-protocol push is popped; the last screen is the caller's |
| terminal-modes | `resize_rewraps_history_without_losing_rows` | Resizing to 60x18 and 140x40 keeps the message and answer, rewraps rows, and never draws wider than the pane |
| composer | `composer_edits_multiline_and_stores_a_large_paste` | `Ctrl+J` newline editing, backspace, a five-line bracketed paste stored as an attachment, and the exact prompt text the model receives |
| composer | `file_mention_inserts_a_workspace_link` | The `@` picker lists the `--cwd` workspace and inserts the chosen file into the prompt |
| commands | `slash_palette_lists_curated_commands_and_settings` | The palette's curated order and filter, the Settings panel, and `/keys` output |
| runs | `streaming_status_reports_and_interrupts` | The status row says `working…`/`esc interrupt`, OSC 7501 reports `working`, `Escape` interrupts with the recorded reason, and the status settles |
| input-queue | `queue_holds_steers_and_deletes` | Held messages in queue mode, the `Queue (n)` strip, `/queue`, `x` deletion, `Enter` steer, and the applied row in the transcript |
| approvals | `approval_panel_allows_one_command` | The permission panel names the tool and command, `y` allows once, the command output returns to the model, and the status is blocked before the answer |
| approvals | `question_panel_answer_reaches_the_next_request` | The question panel's options, the review step, and the chosen label in the tool result the model receives |
| tasks | `background_task_row_and_tasks_panel` | A background command's task row, `1 in background`, the Background tasks panel, and stopping it with `x` |
| daemon | `daemon_stop_reports_disconnect_and_resume_replays_history` | A stopped daemon keeps the draft, names `dimcode --resume`, exits cleanly with the terminal restored; the resumed session replays history and accepts a new prompt |
| launcher | `npm_launcher_streams_and_passes_interrupts` | The npm launcher starts the native TUI, streams an answer, and a second `Ctrl+C` exits 130 with the terminal restored |

## Effects pane text cannot show

The tmux server taps each pane's output stream with `pipe-pane`, writing it to
`<case>/<session>.raw`. A scenario reads that stream for the escapes a captured
screen cannot show:

| Effect | Evidence in the raw stream |
| --- | --- |
| Alternate-screen entry and exit | `CSI ? 1049 h` before the fullscreen composer, `CSI ? 1049 l` when leaving |
| Cursor restoration | `CSI ? 25 h` after exit or after the daemon leaves |
| Keyboard protocol stacks | `CSI > n u` pushes and `CSI < u` pops balance across mode switches and exit |
| Program Status Protocol | `OSC 7501` records: `state=idle`, `state=working`, `state=blocked:kind=permission`, and `state=clear` on exit |
| Process exit status and signals | `pane_dead_status` from `remain-on-exit` (0 for a normal exit, 130 after two interrupts) |

Inside tmux the OSC 7501 records stop at tmux and are not forwarded to the outer
terminal; the app writes them to the pane, so the tap sees them while the outer
terminal does not. Terminal-negotiation behavior that tmux cannot exhibit — the
startup color/DA probe, kitty flag stacks on a negotiating terminal, and the
theme choice from the terminal background — stays at the component level in
`crates/dimcode/tests/cli_flow/exit.rs` and the `tui_default_theme_*` flow.

## Coverage mapping

| Required behavior | tmux scenario | Also asserted by |
| --- | --- | --- |
| Scrollback and fullscreen | `scrollback_streams_and_exits_cleanly`, `fullscreen_switches_screens_and_restores_the_terminal` | `cli_flow/exit.rs`, `dim-cli` render tests |
| Mode switching | `fullscreen_switches_screens_and_restores_the_terminal` | `exit::keyboard_protocol_follows_the_screen_and_restores_both_stacks_on_exit` |
| Resizing | `resize_rewraps_history_without_losing_rows` | `tui_resizing_during_thinking_preserves_multiline_history` (CI) |
| History and resume | `daemon_stop_reports_disconnect_and_resume_replays_history` | `catalog::tui_resume_*` (CI) |
| Normal exit and interruption | `scrollback_streams_and_exits_cleanly`, `npm_launcher_streams_and_passes_interrupts`, `streaming_status_reports_and_interrupts` | `exit::ctrl_c_*` (CI) |
| Composer editing and paste | `composer_edits_multiline_and_stores_a_large_paste` | `interactive_composer_preserves_multiline_pastes_and_edits_atomic_markers` (CI) |
| File mentions and attachments | `file_mention_inserts_a_workspace_link`, `composer_edits_multiline_and_stores_a_large_paste` | `tui_at_mentions_pick_files_from_the_working_directory`, paste flows (CI) |
| Slash commands and panels | `slash_palette_lists_curated_commands_and_settings` | palette and panel flows in `session_management`, `skills`, `workflow`, `mcp` (CI) |
| Streaming and status | `streaming_status_reports_and_interrupts`, `scrollback_streams_and_exits_cleanly` | `tui_reports_the_run_state_to_the_terminal_and_clears_it_on_exit` (CI) |
| Queue controls | `queue_holds_steers_and_deletes` | `tui_queue_*` (CI) |
| Permissions and questions | `approval_panel_allows_one_command`, `question_panel_answer_reaches_the_next_request` | `permission::*`, `question::*` (CI) |
| Task and subagent controls | `background_task_row_and_tasks_panel` | `tasks::*` (CI); member conversations and stop-all are not yet migrated |
| Daemon disconnect/restart | `daemon_stop_reports_disconnect_and_resume_replays_history` | `daemon_replace::*` (CI) |
| Production launcher and App Server | `npm_launcher_streams_and_passes_interrupts`; every scenario runs the production daemon | ignored production flows in `cli_flow` (CI) |

Subagent controls are the known gap: `tasks::` member conversations, member
approval and stop-all still need a tmux scenario, either by scripting a delegated
member's model turns or by driving a real delegation against the fixture.

## Existing terminal E2E cases

`crates/dimcode/tests/cli_flow*.rs` holds 298 cases that drive the TUI (or the
launcher that passes through to it) through a PTY, spread over 26 modules. They
stay in CI until #1802 returns its dispositions and the remaining behaviors have
tmux scenarios; this document lists each one so a migration PR can name the
survivor.

The states below mean:

- **tmux `scenario`** — the scenario exercises the same user-visible flow end to
  end, so the case is a candidate to move once #1802 accepts it.
- **CI (pending)** — the case still runs in ordinary workspace CI. It stays
  there until its disposition lands and its behavior has a tmux scenario.

**`accounts`** — 3 cases

- CI (pending): `dim_device_login_can_retry_the_model_refresh_from_the_account_panel`, `the_provider_panel_signs_in_through_each_described_namespace`, `a_panel_without_the_sign_in_method_shows_the_unsupported_daemon_error`

**`acp`** — 1 case

- CI (pending): `tui_reports_max_tokens_action_and_accepts_the_next_prompt`

**`catalog`** — 18 cases

- CI (pending): `tui_resume_catalog_pages_searches_and_closes_without_waiting_for_a_read`, `tui_resume_labels_origins_and_shows_acp_sessions_when_asked`, `tui_resume_ctrl_s_widens_to_every_workspace_and_names_the_workspace_of_a_row`, `tui_resume_says_when_the_workspace_has_no_saved_sessions`, `tui_resume_hides_other_workspaces_on_the_summaries_fallback`, `a_cold_ui_leaf_submits_its_first_message_with_a_large_native_catalog` (ignored, production), `tui_resume_toggles_between_the_repository_and_all_workspaces`, `tui_resume_keeps_the_repository_rows_on_the_summaries_fallback`, `tui_resume_names_a_one_worktree_repository`, `tui_resume_draws_the_workspace_while_a_slow_git_runs`, `tui_resume_skips_the_repository_scope_a_daemon_cannot_read`, `tui_resume_ctrl_x_deletes_the_selected_row_and_keeps_the_shown_session`, `tui_resume_esc_leaves_the_picker_while_the_delete_is_in_flight`, `tui_resume_deletes_the_shown_session_and_returns_home`, `tui_resume_delete_keeps_the_row_when_the_daemon_cannot_delete`, `tui_resume_delete_keeps_the_row_on_a_failure_and_refreshes_when_nothing_removed`, `tui_resume_delete_after_the_close_lands_home`, `the_resume_hint_is_drawn_translated`

**`context`** — 12 cases

- CI (pending): `tui_display_details_use_one_typed_reply_and_retry_after_collapse`, `tui_displays_history_and_handles_input_while_context_usage_is_pending`, `tui_discards_a_previous_sessions_delayed_context_report`, `tui_compaction_reports_committed_failures_once_and_keeps_call_errors`, `tui_shows_context_usage_the_hello_does_not_list`, `tui_shows_no_reading_and_keeps_working_when_context_usage_fails`, `tui_usage_shows_the_last_requests_cache_hit_rate`, `tui_usage_rounds_the_cache_hit_rate_down_and_hides_an_impossible_usage`, `tui_usage_shows_the_cache_hit_rate_a_resumed_session_recorded`, `tui_usage_cache_hit_rate_ignores_events_off_main`, `tui_usage_withholds_the_cache_hit_rate_until_main_history_loads`, `tui_context_reading_follows_each_finished_model_request_on_main`

**`exit`** — 3 cases

- tmux `fullscreen_switches_screens_and_restores_the_terminal` — `keyboard_protocol_follows_the_screen_and_restores_both_stacks_on_exit`
- CI (pending): `ctrl_c_ends_the_interface_when_the_terminal_stops_reading_after_the_goodbye_panel`, `ctrl_c_ends_the_interface_when_the_output_queue_is_already_full`

**`goal`** — 4 cases

- CI (pending): `tui_goal_shows_the_state_and_marks_the_status_area`, `tui_goal_acts_on_the_shown_line_and_a_new_line_has_none`, `tui_goal_reports_a_failed_read_and_sends_nothing`, `tui_goal_reads_owner_state_while_the_transcript_window_is_loading`

**`hooks`** — 20 cases

- CI (pending): `hooks_toggle_conflict_reads_the_area_again`, `hooks_edit_writes_what_the_editor_leaves`, `hooks_edit_writes_nothing_when_the_editor_changes_nothing`, `hooks_edit_keeps_the_users_text_when_it_does_not_parse`, `hooks_edit_keeps_the_users_text_when_the_revision_is_stale`, `hooks_edit_keeps_one_file_per_edit`, `hooks_edit_writes_nothing_when_the_editor_exits_non_zero`, `hooks_edit_runs_an_editor_path_with_a_space`, `tui_hooks_panel_lists_the_sources_and_toggles_the_project`, `a_missing_source_shows_its_path_and_says_there_is_no_hooks_file`, `tui_hooks_panel_edits_the_user_document`, `tui_hooks_panel_offers_no_edit_without_a_user_document`, `hooks_edit_and_set_refuse_a_daemon_without_a_user_document`, `tui_hooks_panel_settles_a_write_whose_read_fails`, `tui_hooks_panel_drops_the_draft_when_the_conflict_read_fails`, `tui_hooks_panel_keeps_the_users_text_when_the_write_is_refused`, `tui_hooks_panel_reopens_the_users_text_after_a_refusal`, `tui_hooks_panel_writes_nothing_when_the_editor_exits_non_zero`, `hooks_commands_refuse_a_daemon_that_does_not_advertise_them`, `tui_hooks_panel_says_when_the_daemon_cannot_manage_them`

**`mcp`** — 17 cases

- CI (pending): `resumed_mcp_line_commands_use_the_session_workspace_in_both_clients`, `tui_mcp_panel_lists_manages_and_tests_entries`, `tui_mcp_panel_edits_and_creates_entries`, `tui_mcp_panel_seeds_a_remote_patch_without_stored_values`, `tui_mcp_panel_shows_one_entries_failed_configuration_read`, `tui_mcp_panel_lists_entries_when_the_read_is_absent`, `tui_mcp_panel_keeps_the_users_text_when_the_revision_is_stale`, `tui_mcp_panel_reports_a_refused_write`, `tui_mcp_panel_draws_tool_text_sanitized`, `tui_mcp_panel_shows_a_failed_catalog_read`, `mcp_edit_opens_the_patch_without_stored_values`, `tui_mcp_list_keeps_a_session_state_on_one_line`, `mcp_disconnect_closes_the_entry_connections`, `tui_mcp_panel_keeps_the_users_text_when_a_create_finds_a_conflict`, `tui_mcp_panel_marks_a_broken_entry_invalid_and_offers_removal`, `tui_mcp_panel_removes_the_row_of_the_source_it_names`, `tui_mcp_line_commands_take_the_source_that_picks_the_entry`

**`models`** — 9 cases

- CI (pending): `models_reports_account_discovery_failure_instead_of_an_empty_list`, `tui_connect_disabled_provider_refreshes_its_model_management_panel`, `tui_models_area_keeps_a_refresh_whose_read_fails`, `picker_and_limit_declaration_read_the_same_catalog`, `tui_models_area_opens_and_toggles_a_model`, `tui_models_area_offers_no_toggle_for_an_unavailable_model`, `tui_models_area_keeps_the_panel_on_a_refused_write`, `tui_models_area_rereads_after_a_conflict`, `tui_models_area_reports_a_conflict_whose_reread_fails`

**`permission`** — 9 cases

- CI (pending): `tui_resume_requires_the_explicit_permission_before_attaching`, `tui_permission_flag_overrides_saved_controls_and_invalid_defaults_are_ignored`, `tui_home_remembers_permissions_without_login_and_applies_them_to_the_first_message`, `tui_permissions_panel_lists_command_rules_and_returns`, `tui_permissions_panel_adds_and_removes_command_rules`, `tui_permissions_panel_opens_a_placeholder_template`, `tui_permissions_panel_reports_a_raced_remove`, `tui_permissions_panel_shows_a_rule_written_elsewhere_on_the_next_read`, `tui_permissions_arguments_open_the_panel_and_are_not_ignored`

**`prompt_suggestion`** — 14 cases

- CI (pending): `a_completed_turn_asks_for_the_next_message_and_draws_the_offer`, `a_daemon_that_does_not_list_the_method_is_never_asked`, `a_draft_keeps_the_interface_from_asking`, `a_pending_question_keeps_the_interface_from_asking`, `an_active_goal_keeps_the_interface_from_asking`, `right_takes_the_offer_into_the_draft_without_sending`, `tab_takes_the_offer_in_scrollback`, `tab_keeps_the_fullscreen_focus_and_right_still_takes_the_offer`, `enter_on_an_empty_composer_leaves_the_offer`, `escape_and_an_edit_withdraw_the_offer`, `a_narrow_terminal_truncates_the_offer`, `the_pause_is_announced_once_per_session`, `settings_shows_no_suggestion_row_without_the_methods`, `settings_toggles_the_switch_through_the_daemon`

**`prompts`** — 1 case

- CI (pending): `prompt_edit_writes_what_the_editor_leaves`

**`protocol_version`** — 2 cases

- CI (pending): `a_resumed_session_of_a_newer_protocol_names_both_versions_and_offers_no_retry`, `a_resumed_session_of_an_older_protocol_opens` (ignored, production)

**`question`** — 8 cases

- tmux `question_panel_answer_reaches_the_next_request` — `native_tui_question_answer_reaches_the_next_model_request` (ignored, production boundary)
- CI (pending): `tui_questions_keep_structured_answers_and_the_leader_draft`, `tui_questions_retry_and_keep_call_identity_across_late_replies`, `tui_questions_restore_a_child_and_interrupt_its_branch`, `tui_member_questions_stay_read_only_and_a_leader_question_takes_the_prompt`, `tui_commands_run_over_a_pending_question`, `tui_command_arguments_run_over_a_pending_question`, `tui_free_text_answers_keep_their_slashes_and_the_palette_survives_a_dropped_question`

**`scheduled`** — 14 cases

- CI (pending): `user_queue_reports_a_daemon_without_block_presence`, `automatic_inputs_stay_out_of_the_user_queue_after_resume_and_live_delivery`, `scheduled_edit_writes_what_the_editor_leaves`, `tui_scheduled_panel_lists_and_manages_jobs`, `tui_scheduled_panel_edits_and_creates_jobs`, `tui_scheduled_panel_keeps_the_users_text_when_the_revision_is_stale`, `tui_scheduled_panel_reports_a_conflict_on_remove`, `tui_scheduled_panel_shows_every_key_and_confirmation_at_80_columns`, `tui_scheduled_panel_keeps_the_users_text_when_a_create_is_refused`, `tui_scheduled_panel_keeps_a_refused_edit_for_its_own_job`, `scheduled_draws_a_job_name_on_one_line`, `tui_scheduled_panel_reports_an_unreadable_create_reply_as_applied`, `tui_scheduled_panel_says_when_the_daemon_cannot_manage_it`, `a_held_scheduled_input_is_left_by_send_now_and_by_escape_then_enter`

**`session_management`** — 7 cases

- CI (pending): `tui_rename_sets_and_clears_the_title_without_submitting_it`, `tui_palette_rename_waits_for_the_title`, `tui_rename_alone_is_refused_with_the_needs_notice`, `tui_rename_during_a_running_turn_is_a_command_not_a_queued_input`, `tui_rename_without_a_session_sends_nothing`, `tui_rename_reports_a_daemon_without_the_method`, `session_cleanup_asks_on_a_terminal`

**`skills`** — 5 cases

- CI (pending): `skill_discovery_reports_failures_and_a_failed_reload_keeps_the_palette`, `tui_skills_panel_reads_again_after_a_failed_read`, `tui_skills_panel_lists_toggles_and_reloads`, `tui_skills_panel_reloads_an_empty_catalog`, `tui_skills_panel_says_when_the_daemon_cannot_manage_them`

**`staged_update`** — 1 case

- CI (pending): `an_interrupted_check_ends_every_process_it_started`

**`startup`** — 23 cases

- tmux `scrollback_streams_and_exits_cleanly` — `home_browsing_and_exit_need_no_session_in_both_tui_modes`
- CI (pending): `first_shell_command_creates_once_without_submitting_a_model_turn`, `home_previews_the_model_without_creating_a_session_and_survives_preview_errors`, `bare_effort_at_home_lists_the_preview_models_choices`, `bare_effort_at_home_without_a_model_says_what_to_do`, `a_choice_at_home_reaches_the_first_session_and_saves_nothing`, `a_model_picked_at_home_drops_the_effort_home_chose`, `home_that_inherits_a_model_refuses_a_preference_choice`, `fast_at_home_lists_tiers_and_holds_a_choice_for_the_first_session`, `first_text_or_draft_creates_once_with_startup_options_in_both_tui_modes`, `a_refused_leaf_start_shows_the_error_restores_the_terminal_and_exits`, `direct_startup_reaches_each_creation_entry_without_persisting_secrets`, `resume_startup_is_adopted_or_explicitly_rejected_before_input_in_every_client`, `home_first_message_inherits_the_adopted_model_without_replaying_startup`, `temporary_model_preferences_use_adopted_choices_without_a_provider_catalog`, `home_creation_uses_the_advertised_product_or_configured_model`, `a_first_prompt_is_submitted_once_and_the_interface_stays`, `a_first_prompt_submits_its_image_with_the_text`, `a_first_prompt_on_a_resumed_session_submits_into_it`, `a_first_prompt_that_names_a_command_runs_it`, `a_first_prompt_without_a_session_stays_in_the_draft`, `a_first_prompt_is_kept_when_the_resumed_session_cannot_attach`, `a_first_prompt_a_new_draft_keeps_out_is_reported`

**`tasks`** — 33 cases

- CI (pending): `tui_shows_the_transcript_and_submits_input_before_owner_inventory_finishes`, `opening_a_discovered_line_loads_its_transcript_through_the_task_history_queue`, `native_tui_stop_all_settles_the_member_and_reports_cancellation_to_the_leader` (ignored, production), `tui_task_history_resume_windows_complete_main_pages_and_preserves_the_draft`, `tui_titled_silent_command_details_keep_the_command`, `tui_fullscreen_resize_cancels_a_task_release_before_reflow`, `tui_task_inventory_restores_branch_pages_and_retries_only_on_reopen`, `tui_task_inventory_discards_old_pages_and_queued_retries_after_resuming_the_same_session`, `tui_task_stop_all_keeps_its_finite_targets_and_waits_only_for_replies`, `tui_task_stop_all_excludes_delayed_discovery_and_settles_failed_branch_reads`, `tui_task_controls_queued_before_a_replacement_never_retarget_it`, `tui_task_stop_all_keeps_admitted_targets_after_a_session_switch`, `tui_task_end_restored_by_history_is_not_notified_but_a_live_one_is`, `tui_task_inventory_keeps_fifty_terminal_facts_and_their_original_results`, `tui_member_conversation_is_read_only_and_preserves_the_leader_draft`, `tui_member_conversation_settles_old_attach_or_page_before_observing_a_replacement`, `tui_member_conversation_keeps_errors_local_and_reopens_from_a_fresh_history`, `tui_member_conversation_yields_to_a_leader_approval_and_cleans_up_its_held_page`, `tui_main_history_failure_is_visible_and_disables_submission`, `tui_workflow_runs_are_discovered_from_facts_and_stopped_through_their_owner`, `tui_workflow_inventory_errors_block_stop_all_until_reopen_and_stops_follow_other_targets`, `tui_reused_temporary_agents_keep_their_names_after_a_public_directory_read`, `tui_member_cards_share_observations_and_show_the_members_actual_model`, `tui_member_cards_attach_only_unfinished_members_and_keep_a_failed_read_unavailable`, `tui_member_cards_read_agent_names_once_for_ids_the_list_does_not_return`, `tui_member_cards_name_a_native_agent_the_hello_does_not_announce`, `tui_earlier_member_cards_use_cached_and_newly_discovered_agent_names`, `tui_member_cards_keep_the_id_when_the_shared_agent_list_read_fails`, `tui_rejected_delegations_are_not_agents_the_leader_worked_with`, `tui_new_creates_the_session_on_the_first_message`, `tui_page_up_at_the_top_draws_the_branch_from_its_first_message`, `earlier_on_an_ended_line_keeps_the_oldest_loaded_message`, `tui_resume_restores_settled_delegations_outside_the_window_in_dispatch_order`

**`title`** — 1 case

- CI (pending): `titled_attach`

**`update_entry`** — 1 case

- CI (pending): `update_names_the_command_and_sends_no_input`

**`windows`** — 2 cases

- CI (pending): `an_orphan_tool_result_reads_its_arguments_only_after_expanding_details`, `tui_attach_keeps_buffered_configuration_and_skips_ended_branch_inventories`

**`workflow`** — 13 cases

- CI (pending): `workflow_edit_set_and_history_find_a_disabled_project_definition`, `tui_workflow_panel_lists_definitions`, `tui_workflow_save_command_writes_a_definition`, `tui_workflow_panel_starts_a_run_with_parameters`, `tui_workflow_panel_keeps_a_runs_parameters_for_its_own_definition`, `tui_workflow_run_leaves_a_listed_default_unsent`, `tui_workflow_panel_starts_a_run_on_the_shown_line`, `tui_workflow_starts_without_a_session_and_reports_nowhere`, `tui_workflow_panel_reports_a_conflict_and_rereads`, `tui_workflow_panel_refuses_a_rename_and_keeps_the_draft`, `tui_workflow_save_is_refused_when_the_daemon_cannot_write`, `tui_workflow_save_reports_an_invalid_definition_and_keeps_the_draft`, `tui_workflow_panel_offers_no_write_without_the_settings_methods`

**`worktrees`** — 1 case

- CI (pending): `managed_worktree_tui_uses_native_ownership_across_restart_and_cleanup` (ignored, production)

**`cli_flow`** — 76 cases

- tmux `resize_rewraps_history_without_losing_rows` — `tui_resizing_during_thinking_preserves_multiline_history`
- tmux `file_mention_inserts_a_workspace_link` — `tui_at_mentions_pick_files_from_the_working_directory`
- tmux `npm_launcher_streams_and_passes_interrupts` — `unix_npm_wrapper_preserves_the_native_terminal_and_interrupt_flow` (ignored, production boundary)
- tmux `queue_holds_steers_and_deletes` — `tui_queue_panel_deletes_steers_and_moves_applied_inputs_to_the_transcript`
- tmux `queue_holds_steers_and_deletes` — `tui_rapid_queue_controls_settle_each_held_input_once`
- tmux `queue_holds_steers_and_deletes` — `tui_queue_shows_held_messages_and_steers_or_withdraws_them`
- tmux `background_task_row_and_tasks_panel` — `tui_tasks_panel_stops_the_selected_command_or_member`
- tmux `streaming_status_reports_and_interrupts` — `tui_reports_the_run_state_to_the_terminal_and_clears_it_on_exit`
- CI (pending): `piped_prompt_runs_once_and_model_failure_has_a_distinct_exit_code`, `explicit_model_selection_uses_protocol_field_names_from_tui_and_line_client`, `fullscreen_session_attached_before_the_first_frame_shows_one_banner`, `the_model_label_shows_the_adopted_reasoning_effort_in_both_modes`, `a_new_session_shows_the_effort_its_creation_adopted`, `the_model_label_shows_a_configured_profile_effort` (ignored, production), `home_after_new_drops_the_previous_sessions_effort`, `repicking_the_current_model_keeps_the_effort`, `a_resumed_session_draws_the_adopted_model_and_effort`, `another_clients_adoption_redraws_the_model_label`, `a_narrow_composer_drops_the_effort_before_the_model`, `tui_escape_sends_the_reason_that_records_the_interruption`, `tui_shell_mode_runs_through_session_methods_and_shows_the_recorded_run`, `tui_effort_dialog_selects_the_current_models_effort_and_cancels_without_writing`, `tui_mcp_and_configuration_commands_use_session_methods`, `tui_mcp_without_a_session_reports_the_catalog_only`, `tui_agent_without_a_session_answers_the_no_session_notice_and_sends_nothing`, `tui_valued_effort_at_home_without_a_model_says_what_to_do_and_sends_nothing`, `tui_shell_session_creation_failure_settles_with_the_no_session_notice`, `tui_pasted_images_and_long_text_submit_as_user_content`, `tui_skill_rows_come_from_the_catalog_and_complete_without_sending`, `reloading_skills_changes_the_palette_rows`, `a_daemon_without_skill_catalog_keeps_a_working_command_palette`, `tui_palette_tab_only_completes_and_enter_still_runs`, `tui_sends_dropped_files_as_links_and_images_by_model_capability`, `fullscreen_login_releases_mouse_for_the_link_and_restores_the_preference_on_close`, `tui_mcp_says_when_no_server_is_configured`, `tui_mcp_shows_failure_reasons_rejections_and_the_catalog_error`, `tui_mcp_enables_and_disables_a_project_server`, `tui_web_toggles_the_daemon_switch_without_leaving`, `tui_stays_up_when_the_daemon_leaves_and_restores_the_terminal_on_exit`, `tui_provider_management_uses_the_production_daemon_and_survives_restart` (ignored, production), `tui_registers_with_the_ui_registry_and_leaves_when_asked`, `tui_export_and_queue_require_an_open_session`, `tui_export_dialog_matches_the_subcommand_and_cancel_sends_nothing`, `tui_queue_recall_refuses_attachments_and_steering_preserves_all_content`, `tui_ctrl_enter_interrupts_the_run_and_starts_a_new_one_with_the_draft`, `tui_ctrl_s_steers_the_draft_while_a_run_is_in_progress`, `tui_escape_withdraws_the_queue_before_the_run_stops`, `tui_command_output_refreshes_one_read_and_preserves_its_branch_and_terminal_result`, `tui_command_output_errors_stop_refresh_and_reopening_retries`, `tui_permissions_keep_pending_file_and_member_questions_and_save_command_rules`, `tui_approval_alias_selects_presets_and_remembers_them`, `tui_shift_tab_cycles_permissions_and_preserves_the_draft_in_both_modes`, `tui_yields_the_title_and_the_bar_to_a_terminal_that_keeps_the_record`, `tui_composer_shows_context_use_and_refreshes_it_when_a_run_ends`, `tui_compaction_row_runs_then_shows_the_summary_it_recorded`, `tui_compact_displays_committed_outcomes_and_reports_an_unsupported_daemon`, `tui_context_warning_starts_at_the_threshold_share_and_a_compaction_refreshes_it`, `cold_hosted_tui_opens_without_reading_the_session_catalog`, `resume_by_id_takes_its_model_from_the_attach_record`, `resume_by_id_finds_the_model_in_the_catalog_when_the_attach_reply_is_empty`, `tui_rebuilds_compaction_rows_from_history_on_resume`, `tui_draws_a_prompt_hooks_context_as_a_status_row`, `tui_draws_a_stop_hooks_follow_up_as_one_row`, `tui_names_a_one_line_hook_context_in_the_singular`, `tui_draws_an_operators_own_hook_marker_verbatim`, `tui_provider_create_recovery_preserves_changed_draft_until_confirmation`, `tui_models_picker_edits_only_the_subagent_model_policy`, `tui_models_picker_hides_manage_models_without_management_methods`, `tui_models_picker_cycles_reasoning_effort`, `tui_models_picker_writes_the_default_the_launch_workspace_reads`, `tui_models_picker_draws_its_hint_in_the_interface_language`, `tui_models_picker_reads_pages_on_demand_and_reopens_without_a_cache`, `tui_models_picker_names_every_provider_with_no_selectable_model`, `member_tool_approval_requires_trusted_details_in_both_terminal_modes`, `member_approval_without_the_detail_method_offers_only_rejection`, `tui_default_theme_respects_background_and_saved_choice`

Modules: 26; cases: 298; ignored: 8.
