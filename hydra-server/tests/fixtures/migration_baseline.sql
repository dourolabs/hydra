-- baseline-version: 20260601000000
-- migrations-hash: bbbc5e1fe2094708ba40c41d62d03b667026fe54a397229cb9f1962a7ec526be

--
-- PostgreSQL database dump
--


-- Dumped from database version 16.14 (Debian 16.14-1.pgdg12+1)
-- Dumped by pg_dump version 16.14 (Debian 16.14-1.pgdg12+1)

SET statement_timeout = 0;
SET lock_timeout = 0;
SET idle_in_transaction_session_timeout = 0;
SET client_encoding = 'SQL_ASCII';
SET standard_conforming_strings = on;
SELECT pg_catalog.set_config('search_path', '', false);
SET check_function_bodies = false;
SET xmloption = content;
SET client_min_messages = warning;
SET row_security = off;

--
-- Data for Name: actors; Type: TABLE DATA; Schema: metis; Owner: postgres
--



--
-- Data for Name: actors_v2; Type: TABLE DATA; Schema: metis; Owner: postgres
--

INSERT INTO metis.actors_v2 (id, version_number, auth_token_hash, auth_token_salt, actor_id, created_at, updated_at, creator, actor, is_latest) VALUES ('users/alice', 1, '', '', '{"kind": "user", "name": "alice"}', '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00', 'alice', '{"System": {"worker_name": "seed-migration-fixture", "on_behalf_of": null}}', true) ON CONFLICT DO NOTHING;


--
-- Data for Name: agents; Type: TABLE DATA; Schema: metis; Owner: postgres
--

INSERT INTO metis.agents (name, prompt_path, max_tries, max_simultaneous, is_assignment_agent, deleted, created_at, updated_at, secrets, mcp_config_path, is_default_conversation_agent) VALUES ('reviewer', 'prompts/reviewer.md', 3, 1, false, false, '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00', '[]', NULL, false) ON CONFLICT DO NOTHING;


--
-- Data for Name: auth_tokens; Type: TABLE DATA; Schema: metis; Owner: postgres
--

INSERT INTO metis.auth_tokens (actor_name, token_hash, created_at, session_id, is_revoked) VALUES ('users/alice', 'legacy-c5020d78d2697962f5cca992b105973a', '2026-01-01 00:00:00+00', NULL, false) ON CONFLICT DO NOTHING;
INSERT INTO metis.auth_tokens (actor_name, token_hash, created_at, session_id, is_revoked) VALUES ('users/alice', 'session-609cf34fa529c3258c196ceddcd26a00', '2026-01-01 00:00:00+00', 's-xvzvsvzjwf', false) ON CONFLICT DO NOTHING;


--
-- Data for Name: conversation_events_v2; Type: TABLE DATA; Schema: metis; Owner: postgres
--

INSERT INTO metis.conversation_events_v2 (id, conversation_id, version_number, event_type, event_data, actor, created_at) VALUES (1, 'c-yrywlaekfg', 1, 'suspending', '{"type": "suspending", "reason": "suspend-uhujoez", "timestamp": "2026-01-01T00:00:00Z"}', '{"System": {"worker_name": "seed-migration-fixture", "on_behalf_of": null}}', '2026-01-01 00:00:00+00') ON CONFLICT DO NOTHING;
INSERT INTO metis.conversation_events_v2 (id, conversation_id, version_number, event_type, event_data, actor, created_at) VALUES (2, 'c-yrywlaekfg', 2, 'suspending', '{"type": "suspending", "reason": "suspend-wclur", "timestamp": "2026-01-01T00:00:00Z"}', '{"System": {"worker_name": "seed-migration-fixture", "on_behalf_of": null}}', '2026-01-01 00:00:00+00') ON CONFLICT DO NOTHING;
INSERT INTO metis.conversation_events_v2 (id, conversation_id, version_number, event_type, event_data, actor, created_at) VALUES (3, 'c-yrywlaekfg', 3, 'suspending', '{"type": "suspending", "reason": "suspend-ttupdehrj", "timestamp": "2026-01-01T00:00:00Z"}', '{"System": {"worker_name": "seed-migration-fixture", "on_behalf_of": null}}', '2026-01-01 00:00:00+00') ON CONFLICT DO NOTHING;


--
-- Data for Name: conversations_v2; Type: TABLE DATA; Schema: metis; Owner: postgres
--

INSERT INTO metis.conversations_v2 (id, version_number, title, agent_name, status, creator, deleted, actor, is_latest, created_at, updated_at, session_settings) VALUES ('c-yrywlaekfg', 1, 'convo-salypj', 'reviewer', 'active', 'alice', false, '{"System": {"worker_name": "seed-migration-fixture", "on_behalf_of": null}}', true, '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00', '{}') ON CONFLICT DO NOTHING;


--
-- Data for Name: documents; Type: TABLE DATA; Schema: metis; Owner: postgres
--



--
-- Data for Name: documents_v2; Type: TABLE DATA; Schema: metis; Owner: postgres
--

INSERT INTO metis.documents_v2 (id, version_number, title, body_markdown, path, deleted, created_at, updated_at, actor, is_latest) VALUES ('d-bbqedamtkg', 1, 'doc-mtksf', '# seed-0

body-mjkxb
', '/notes/seed-0000.md', false, '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00', '{"System": {"worker_name": "seed-migration-fixture", "on_behalf_of": null}}', true) ON CONFLICT DO NOTHING;


--
-- Data for Name: issues; Type: TABLE DATA; Schema: metis; Owner: postgres
--



--
-- Data for Name: issues_v2; Type: TABLE DATA; Schema: metis; Owner: postgres
--

INSERT INTO metis.issues_v2 (id, version_number, issue_type, description, creator, progress, status, assignee, job_settings, deleted, created_at, updated_at, actor, title, is_latest, form, form_response, feedback, assignee_principal) VALUES ('i-tzcvpqdtmm', 1, 'task', 'described-njqd', 'alice', '', 'open', 'users/alice', '{}', false, '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00', '{"System": {"worker_name": "seed-migration-fixture", "on_behalf_of": null}}', 'issue-ugxmq', true, NULL, NULL, NULL, '{"kind": "user", "name": "alice"}') ON CONFLICT DO NOTHING;
INSERT INTO metis.issues_v2 (id, version_number, issue_type, description, creator, progress, status, assignee, job_settings, deleted, created_at, updated_at, actor, title, is_latest, form, form_response, feedback, assignee_principal) VALUES ('i-ollfchkixw', 1, 'task', 'described-uffk', 'alice', '', 'open', 'agents/reviewer', '{}', false, '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00', '{"System": {"worker_name": "seed-migration-fixture", "on_behalf_of": null}}', 'issue-dihjxazm', true, NULL, NULL, NULL, '{"kind": "agent", "name": "reviewer"}') ON CONFLICT DO NOTHING;
INSERT INTO metis.issues_v2 (id, version_number, issue_type, description, creator, progress, status, assignee, job_settings, deleted, created_at, updated_at, actor, title, is_latest, form, form_response, feedback, assignee_principal) VALUES ('i-sekokuqtjt', 1, 'task', 'described-azhrf', 'alice', '', 'open', 'external/linear/HYDRA-123', '{}', false, '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00', '{"System": {"worker_name": "seed-migration-fixture", "on_behalf_of": null}}', 'issue-qbospw', true, NULL, NULL, NULL, '{"kind": "external", "system": "linear", "username": "HYDRA-123"}') ON CONFLICT DO NOTHING;
INSERT INTO metis.issues_v2 (id, version_number, issue_type, description, creator, progress, status, assignee, job_settings, deleted, created_at, updated_at, actor, title, is_latest, form, form_response, feedback, assignee_principal) VALUES ('i-rckrzyecnk', 1, 'task', 'described-dvytooj', 'alice', '', 'open', NULL, '{}', false, '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00', '{"System": {"worker_name": "seed-migration-fixture", "on_behalf_of": null}}', 'issue-zrjfaygrn', true, NULL, NULL, NULL, NULL) ON CONFLICT DO NOTHING;
INSERT INTO metis.issues_v2 (id, version_number, issue_type, description, creator, progress, status, assignee, job_settings, deleted, created_at, updated_at, actor, title, is_latest, form, form_response, feedback, assignee_principal) VALUES ('i-clgshrveps', 1, 'task', 'described-uvsjyirvl', 'alice', '', 'open', 'users/alice', '{}', false, '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00', '{"System": {"worker_name": "seed-migration-fixture", "on_behalf_of": null}}', 'issue-tfyjqxyhf', true, NULL, NULL, NULL, '{"kind": "user", "name": "alice"}') ON CONFLICT DO NOTHING;


--
-- Data for Name: labels; Type: TABLE DATA; Schema: metis; Owner: postgres
--



--
-- Data for Name: label_associations; Type: TABLE DATA; Schema: metis; Owner: postgres
--



--
-- Data for Name: migration_status; Type: TABLE DATA; Schema: metis; Owner: postgres
--



--
-- Data for Name: object_relationships; Type: TABLE DATA; Schema: metis; Owner: postgres
--

INSERT INTO metis.object_relationships (source_id, source_kind, target_id, target_kind, rel_type, created_at) VALUES ('i-tzcvpqdtmm', 'issue', 'i-ollfchkixw', 'issue', 'child-of', '2026-01-01 00:00:00+00') ON CONFLICT DO NOTHING;
INSERT INTO metis.object_relationships (source_id, source_kind, target_id, target_kind, rel_type, created_at) VALUES ('i-tzcvpqdtmm', 'issue', 'p-ydxblndpne', 'patch', 'has-patch', '2026-01-01 00:00:00+00') ON CONFLICT DO NOTHING;
INSERT INTO metis.object_relationships (source_id, source_kind, target_id, target_kind, rel_type, created_at) VALUES ('i-tzcvpqdtmm', 'issue', 'd-bbqedamtkg', 'document', 'refers-to', '2026-01-01 00:00:00+00') ON CONFLICT DO NOTHING;


--
-- Data for Name: patches; Type: TABLE DATA; Schema: metis; Owner: postgres
--



--
-- Data for Name: patches_v2; Type: TABLE DATA; Schema: metis; Owner: postgres
--

INSERT INTO metis.patches_v2 (id, version_number, title, description, diff, status, is_automatic_backup, reviews, service_repo_name, github, deleted, created_at, updated_at, branch_name, commit_range, actor, creator, base_branch, is_latest) VALUES ('p-ydxblndpne', 1, 'patch-rdoj', 'patch desc-mikkjyfkq', '', 'open', false, '[{"author": {"kind": "user", "name": "alice"}, "contents": "looks good", "is_approved": true, "submitted_at": "2026-01-01T00:00:00Z"}]', 'dourolabs/hydra', NULL, false, '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00', 'seed/p-ydxblndpne', NULL, '{"System": {"worker_name": "seed-migration-fixture", "on_behalf_of": null}}', 'alice', 'main', true) ON CONFLICT DO NOTHING;
INSERT INTO metis.patches_v2 (id, version_number, title, description, diff, status, is_automatic_backup, reviews, service_repo_name, github, deleted, created_at, updated_at, branch_name, commit_range, actor, creator, base_branch, is_latest) VALUES ('p-pwfnyaklyv', 1, 'patch-dazepxa', 'patch desc-gqtiv', '', 'open', false, '[{"author": {"kind": "agent", "name": "reviewer"}, "contents": "looks good", "is_approved": true, "submitted_at": "2026-01-01T00:00:00Z"}]', 'dourolabs/hydra', NULL, false, '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00', 'seed/p-pwfnyaklyv', NULL, '{"System": {"worker_name": "seed-migration-fixture", "on_behalf_of": null}}', 'alice', 'main', true) ON CONFLICT DO NOTHING;
INSERT INTO metis.patches_v2 (id, version_number, title, description, diff, status, is_automatic_backup, reviews, service_repo_name, github, deleted, created_at, updated_at, branch_name, commit_range, actor, creator, base_branch, is_latest) VALUES ('p-uhwpbknxrs', 1, 'patch-wfkojb', 'patch desc-jmou', '', 'open', false, '[{"author": {"kind": "user", "name": "bob"}, "contents": "looks good", "is_approved": false, "submitted_at": "2026-01-01T00:00:00Z"}]', 'dourolabs/hydra', NULL, false, '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00', 'seed/p-uhwpbknxrs', NULL, '{"System": {"worker_name": "seed-migration-fixture", "on_behalf_of": null}}', 'alice', 'main', true) ON CONFLICT DO NOTHING;


--
-- Data for Name: payload_schema_versions; Type: TABLE DATA; Schema: metis; Owner: postgres
--

INSERT INTO metis.payload_schema_versions (object_type, current_version, created_at, updated_at) VALUES ('issue', 1, '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00') ON CONFLICT DO NOTHING;
INSERT INTO metis.payload_schema_versions (object_type, current_version, created_at, updated_at) VALUES ('patch', 1, '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00') ON CONFLICT DO NOTHING;
INSERT INTO metis.payload_schema_versions (object_type, current_version, created_at, updated_at) VALUES ('task', 1, '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00') ON CONFLICT DO NOTHING;
INSERT INTO metis.payload_schema_versions (object_type, current_version, created_at, updated_at) VALUES ('task_status_log', 1, '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00') ON CONFLICT DO NOTHING;
INSERT INTO metis.payload_schema_versions (object_type, current_version, created_at, updated_at) VALUES ('repository', 1, '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00') ON CONFLICT DO NOTHING;
INSERT INTO metis.payload_schema_versions (object_type, current_version, created_at, updated_at) VALUES ('actor', 3, '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00') ON CONFLICT DO NOTHING;
INSERT INTO metis.payload_schema_versions (object_type, current_version, created_at, updated_at) VALUES ('user', 3, '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00') ON CONFLICT DO NOTHING;
INSERT INTO metis.payload_schema_versions (object_type, current_version, created_at, updated_at) VALUES ('document', 1, '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00') ON CONFLICT DO NOTHING;


--
-- Data for Name: repositories; Type: TABLE DATA; Schema: metis; Owner: postgres
--



--
-- Data for Name: repositories_v2; Type: TABLE DATA; Schema: metis; Owner: postgres
--

INSERT INTO metis.repositories_v2 (id, version_number, remote_url, default_branch, default_image, created_at, updated_at, deleted, actor, is_latest, merge_policy) VALUES ('dourolabs/hydra', 1, 'https://github.com/dourolabs/hydra', 'main', NULL, '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00', false, '{"System": {"worker_name": "seed-migration-fixture", "on_behalf_of": null}}', true, NULL) ON CONFLICT DO NOTHING;


--
-- Data for Name: session_events_v2; Type: TABLE DATA; Schema: metis; Owner: postgres
--



--
-- Data for Name: session_state_v2; Type: TABLE DATA; Schema: metis; Owner: postgres
--



--
-- Data for Name: task_status_logs; Type: TABLE DATA; Schema: metis; Owner: postgres
--



--
-- Data for Name: tasks; Type: TABLE DATA; Schema: metis; Owner: postgres
--



--
-- Data for Name: tasks_v2; Type: TABLE DATA; Schema: metis; Owner: postgres
--

INSERT INTO metis.tasks_v2 (id, version_number, spawned_from, image, env_vars, cpu_limit, memory_limit, status, last_message, error, deleted, created_at, updated_at, actor, creator, secrets, creation_time, start_time, end_time, is_latest, conversation_id, usage, mount_spec, agent_config, mode, resumed_from) VALUES ('s-xvzvsvzjwf', 1, NULL, NULL, '{}', NULL, NULL, 'complete', NULL, NULL, false, '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00', '{"System": {"worker_name": "seed-migration-fixture", "on_behalf_of": null}}', 'alice', NULL, '2026-01-01 00:00:00+00', NULL, NULL, true, 'c-yrywlaekfg', NULL, '{"mounts": [{"type": "bundle", "bundle": {"type": "none"}, "target": "repo"}], "working_dir": "repo"}', '{"agent_name": "reviewer"}', '{"type": "interactive", "conversation_id": "c-yrywlaekfg", "idle_timeout_secs": null, "conversation_resume_from": null}', NULL) ON CONFLICT DO NOTHING;
INSERT INTO metis.tasks_v2 (id, version_number, spawned_from, image, env_vars, cpu_limit, memory_limit, status, last_message, error, deleted, created_at, updated_at, actor, creator, secrets, creation_time, start_time, end_time, is_latest, conversation_id, usage, mount_spec, agent_config, mode, resumed_from) VALUES ('s-blwtbryije', 1, NULL, NULL, '{}', NULL, NULL, 'complete', NULL, NULL, false, '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00', '{"System": {"worker_name": "seed-migration-fixture", "on_behalf_of": null}}', 'alice', NULL, '2026-01-01 00:00:00+00', NULL, NULL, true, NULL, NULL, '{"mounts": [{"type": "bundle", "bundle": {"type": "none"}, "target": "repo"}], "working_dir": "repo"}', '{}', '{"type": "headless", "prompt": "headless-0: do-buwno"}', NULL) ON CONFLICT DO NOTHING;


--
-- Data for Name: user_secrets; Type: TABLE DATA; Schema: metis; Owner: postgres
--



--
-- Data for Name: users; Type: TABLE DATA; Schema: metis; Owner: postgres
--



--
-- Data for Name: users_v2; Type: TABLE DATA; Schema: metis; Owner: postgres
--

INSERT INTO metis.users_v2 (id, version_number, username, github_user_id, created_at, updated_at, deleted, actor, is_latest) VALUES ('alice', 1, 'alice', NULL, '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00', false, '{"System": {"worker_name": "seed-migration-fixture", "on_behalf_of": null}}', true) ON CONFLICT DO NOTHING;
INSERT INTO metis.users_v2 (id, version_number, username, github_user_id, created_at, updated_at, deleted, actor, is_latest) VALUES ('bob', 1, 'bob', NULL, '2026-01-01 00:00:00+00', '2026-01-01 00:00:00+00', false, '{"System": {"worker_name": "seed-migration-fixture", "on_behalf_of": null}}', true) ON CONFLICT DO NOTHING;


--
-- Name: conversation_events_v2_id_seq; Type: SEQUENCE SET; Schema: metis; Owner: postgres
--

SELECT pg_catalog.setval('metis.conversation_events_v2_id_seq', 3, true);


--
-- Name: session_events_v2_id_seq; Type: SEQUENCE SET; Schema: metis; Owner: postgres
--

SELECT pg_catalog.setval('metis.session_events_v2_id_seq', 1, false);


--
-- PostgreSQL database dump complete
--


