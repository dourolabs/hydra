//! Generic model dispatch. `ModelSelector` is an enum (not a trait) that
//! routes batch / interactive runs to the per-model wrappers and translates
//! the generic `WorkerInputMessage` / `WorkerEvent` vocabulary into each
//! model's native I/O types inside the `match` arms.
//!
//! See `designs/worker-model-commands-refactor.md` §3 and §6, and
//! `designs/sessions-worker-run-interface.md` §3.1 / §3.2 for the
//! `drive_headless` / `drive_interactive` dispatch surface that owns the
//! three WS phases.

use std::{collections::HashMap, path::PathBuf, time::Duration};

use anyhow::{anyhow, Result};
use futures::{Sink, Stream};
use hydra_common::api::v1::{
    conversations::{ServerMessage, WorkerMessage},
    sessions::SessionEvent,
};
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_tungstenite::tungstenite;

use crate::worker::claude::{Claude, ClaudeEvent, ClaudeResume, ClaudeUserMessage};
use crate::worker::codex::Codex;
use crate::worker::relay_adapter::{spawn_relay_pump, RelayAdapter};
use crate::worker::report::{
    MaterializeError, NativeResume, RunReport, SessionResume, TokenUsage, WorkerEvent,
    WorkerInputMessage,
};
use crate::worker::socket::WorkerSocket;

/// Routes a worker invocation to either the Claude or Codex per-model wrapper.
///
/// Constructed once per worker via [`ModelSelector::from_context`]; the public
/// surface is [`Self::drive_headless`] / [`Self::drive_interactive`], which
/// own all three WS phases (negotiate, first-message, model-stream pump).
pub enum ModelSelector {
    Claude(Claude),
    Codex(Codex),
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Kind {
    Claude,
    Codex,
}

impl ModelSelector {
    /// Build a `ModelSelector` from the per-worker context. Decides between
    /// `Claude` and `Codex` based on the model name (see [`Self::decide_kind`])
    /// and constructs the chosen wrapper, propagating any setup error.
    pub async fn from_context(
        model: &Option<String>,
        working_dir: PathBuf,
        home_dir: PathBuf,
        env: HashMap<String, String>,
        mcp_config_json: Option<&str>,
        idle_timeout: Duration,
    ) -> Result<Self> {
        let kind = Self::decide_kind(model.as_deref());
        match kind {
            Kind::Claude => Ok(Self::Claude(
                Claude::new(
                    model.clone(),
                    working_dir,
                    home_dir,
                    env,
                    mcp_config_json,
                    idle_timeout,
                )
                .await?,
            )),
            Kind::Codex => Ok(Self::Codex(
                Codex::new(model.clone(), working_dir, home_dir, env, mcp_config_json).await?,
            )),
        }
    }

    /// Drive a complete headless (non-interactive) run on `ws`. Owns Phase 1
    /// (context negotiation), Phase 2 (first-message), and the per-wrapper
    /// model invocation; returns the resulting [`RunReport`].
    pub async fn drive_headless<S>(&mut self, mut ws: WorkerSocket<S>) -> Result<RunReport>
    where
        S: Sink<tungstenite::Message, Error = tungstenite::Error>
            + Stream<Item = std::result::Result<tungstenite::Message, tungstenite::Error>>
            + Unpin,
    {
        let (resume, primer) = self.phase1_negotiate(&mut ws).await?;
        let (agent_prompt, user_message) = phase2_ready(&mut ws).await?;
        let prompt = concat_first_message(&primer, &agent_prompt, &user_message);
        self.run_with_native(&prompt, resume).await
    }

    /// Drive a complete interactive run on `ws`. Owns Phase 1 / Phase 2 and
    /// then hands the socket to the [`crate::worker::relay_adapter`] pump for
    /// Phase 3; returns the resulting [`RunReport`].
    pub async fn drive_interactive<S>(&mut self, mut ws: WorkerSocket<S>) -> Result<RunReport>
    where
        S: Sink<tungstenite::Message, Error = tungstenite::Error>
            + Stream<Item = std::result::Result<tungstenite::Message, tungstenite::Error>>
            + Unpin
            + Send
            + 'static,
    {
        let (resume, primer) = self.phase1_negotiate(&mut ws).await?;
        let (agent_prompt, user_message) = phase2_ready(&mut ws).await?;
        let prompt = concat_first_message(&primer, &agent_prompt, &user_message);
        let RelayAdapter {
            input_rx,
            output_tx,
            pump,
        } = spawn_relay_pump(ws);
        let report = self
            .run_interactive_with_native(input_rx, output_tx, &prompt, resume)
            .await;
        let _ = pump.await;
        report
    }

    /// Phase 1 — context negotiation. Sends `Fresh`, awaits `ResumeContext`,
    /// attempts native materialization, and on failure asks for the prior
    /// session's transcript as primer text. Returns the resolved
    /// [`NativeResume`] (if any) and the primer-event list (may be empty).
    async fn phase1_negotiate<S>(
        &mut self,
        ws: &mut WorkerSocket<S>,
    ) -> Result<(Option<NativeResume>, Vec<SessionEvent>)>
    where
        S: Sink<tungstenite::Message, Error = tungstenite::Error>
            + Stream<Item = std::result::Result<tungstenite::Message, tungstenite::Error>>
            + Unpin,
    {
        ws.send(WorkerMessage::Fresh).await?;
        let resume_ctx = ws
            .recv()
            .await?
            .ok_or_else(|| anyhow!("ws closed before ResumeContext"))?;
        let (resume_blob, prior_session_id) = match resume_ctx {
            ServerMessage::ResumeContext {
                resume_blob,
                prior_session_id,
            } => (resume_blob, prior_session_id),
            other => return Err(anyhow!("expected ResumeContext, got {other:?}")),
        };

        // Try native materialization first; on Err, fall back to transcript replay.
        let (native, need_transcript) = match resume_blob {
            Some(bytes) => match self.try_materialize_resume(&bytes).await {
                Ok(native) => (Some(native), false),
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        prior_session_id = ?prior_session_id,
                        "native resume materialization failed; falling back to transcript replay",
                    );
                    (None, prior_session_id.is_some())
                }
            },
            None => (None, prior_session_id.is_some()),
        };

        // Per design §1.4 / §6: when native materialization succeeded the
        // worker emits `SessionEvent::Resumed` exactly once on its session
        // log.
        if native.is_some() {
            if let Some(from) = prior_session_id.clone() {
                ws.send(WorkerMessage::Event {
                    event: SessionEvent::Resumed {
                        from_session_id: from,
                        timestamp: chrono::Utc::now(),
                    },
                })
                .await?;
            }
        }

        let primer = if need_transcript {
            if let Some(prior) = prior_session_id {
                ws.send(WorkerMessage::RequestTranscript {
                    prior_session_id: prior,
                })
                .await?;
                let resp = ws
                    .recv()
                    .await?
                    .ok_or_else(|| anyhow!("ws closed before Transcript"))?;
                match resp {
                    ServerMessage::Transcript { events } => events,
                    other => return Err(anyhow!("expected Transcript, got {other:?}")),
                }
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        Ok((native, primer))
    }

    /// Run one batch turn. The Codex arm currently has no resume support;
    /// `resume` is logged and dropped if supplied.
    async fn run_with_native(
        &mut self,
        prompt: &str,
        resume: Option<NativeResume>,
    ) -> Result<RunReport> {
        match self {
            Self::Claude(c) => {
                let claude_resume = resume.and_then(|r| match r {
                    NativeResume::Claude(c) => Some(c),
                    _ => None,
                });
                c.run(prompt, claude_resume).await
            }
            Self::Codex(c) => {
                if resume.is_some() {
                    tracing::debug!("ModelSelector: dropping `resume` for Codex (out of scope)");
                }
                c.run(prompt).await
            }
        }
    }

    /// Per-wrapper materialization of a resume blob. Returns
    /// `Err(NotImplemented)` for wrappers that don't support resume; the
    /// caller falls back to transcript-primer replay either way.
    async fn try_materialize_resume(
        &self,
        state_bytes: &[u8],
    ) -> std::result::Result<NativeResume, MaterializeError> {
        match self {
            Self::Claude(c) => c.try_materialize(state_bytes),
            Self::Codex(c) => c.try_materialize(state_bytes),
        }
    }

    /// Run one interactive session. The Codex arm always returns `Err`
    /// (`codex` does not have a long-lived stdin/stdout shape); the caller is
    /// expected to kind-check before opening the relay WebSocket so this is a
    /// belt-and-suspenders guard.
    async fn run_interactive_with_native(
        &mut self,
        input_rx: mpsc::Receiver<WorkerInputMessage>,
        output_tx: mpsc::Sender<WorkerEvent>,
        prompt: &str,
        resume: Option<NativeResume>,
    ) -> Result<RunReport> {
        match self {
            Self::Claude(c) => {
                let claude_resume = resume.and_then(|r| match r {
                    NativeResume::Claude(c) => Some(c),
                    _ => None,
                });
                let (claude_in_tx, claude_in_rx) = mpsc::channel::<ClaudeUserMessage>(32);
                let (claude_out_tx, claude_out_rx) = mpsc::channel::<ClaudeEvent>(32);
                let in_pump = spawn_input_translator(input_rx, claude_in_tx);
                let out_pump = spawn_output_translator(claude_out_rx, output_tx);
                let report = c
                    .run_interactive(claude_in_rx, claude_out_tx, prompt, claude_resume)
                    .await;
                let _ = in_pump.await;
                let _ = out_pump.await;
                report
            }
            Self::Codex(_) => Err(anyhow!("model does not support interactive mode")),
        }
    }

    /// Side-effect-free probe over the model name: returns `true` if the name
    /// resolves to a wrapper that supports interactive (long-lived
    /// stdin/stdout) runs, `false` otherwise. Used by `worker_run` to
    /// short-circuit Codex+interactive before constructing the wrapper, which
    /// would otherwise perform per-worker setup (e.g. `codex login`, writing
    /// `~/.codex/config.toml`, creating the output tempdir).
    pub(crate) fn supports_interactive(name: Option<&str>) -> bool {
        matches!(Self::decide_kind(name), Kind::Claude)
    }

    /// Decide which kind of model wrapper this name maps to. Name-based
    /// (per design §6); matches either an exact bare family name or one of
    /// the `<family>-...` prefix forms:
    ///
    /// * `claude` / `haiku` / `sonnet` / `opus` (bare) or `claude-` /
    ///   `haiku-` / `sonnet-` / `opus-` (prefix) → [`Kind::Claude`]
    /// * `gpt-` / `o1` / `o3` / `o4` / `codex-` → [`Kind::Codex`]
    /// * Everything else (and `None`) → [`Kind::Codex`] with a `warn!` log so
    ///   misroutes are spottable.
    fn decide_kind(name: Option<&str>) -> Kind {
        let Some(raw) = name else {
            return Kind::Codex;
        };
        let lc = raw.to_ascii_lowercase();
        let claude_exact = ["claude", "haiku", "sonnet", "opus"];
        let claude_prefixes = ["claude-", "haiku-", "sonnet-", "opus-"];
        let codex_prefixes = ["gpt-", "o1", "o3", "o4", "codex-"];
        if claude_exact.iter().any(|n| lc == *n) {
            return Kind::Claude;
        }
        if claude_prefixes.iter().any(|p| lc.starts_with(p)) {
            return Kind::Claude;
        }
        if codex_prefixes.iter().any(|p| lc.starts_with(p)) {
            return Kind::Codex;
        }
        tracing::warn!(model = %raw, "ModelSelector: model name unrecognized, defaulting to Codex");
        Kind::Codex
    }
}

/// Phase 2 — send `Ready`, await `FirstMessage`, return its contents.
async fn phase2_ready<S>(ws: &mut WorkerSocket<S>) -> Result<(String, String)>
where
    S: Sink<tungstenite::Message, Error = tungstenite::Error>
        + Stream<Item = std::result::Result<tungstenite::Message, tungstenite::Error>>
        + Unpin,
{
    ws.send(WorkerMessage::Ready).await?;
    let msg = ws
        .recv()
        .await?
        .ok_or_else(|| anyhow!("ws closed before FirstMessage"))?;
    match msg {
        ServerMessage::FirstMessage {
            agent_prompt,
            user_message,
        } => Ok((agent_prompt, user_message)),
        other => Err(anyhow!("expected FirstMessage, got {other:?}")),
    }
}

/// Concatenate the optional primer (from a transcript-replay fallback),
/// the agent prompt, and the user message into one model-input string,
/// collapsing empty pieces.
fn concat_first_message(
    primer_events: &[SessionEvent],
    agent_prompt: &str,
    user_message: &str,
) -> String {
    let primer = primer_to_text(primer_events);
    let base = match (agent_prompt, user_message) {
        ("", "") => String::new(),
        ("", u) => u.to_string(),
        (p, "") => p.to_string(),
        (p, u) => format!("{p}\n\n{u}"),
    };
    if primer.is_empty() {
        base
    } else if base.is_empty() {
        primer
    } else {
        format!("{primer}\n\n{base}")
    }
}

fn primer_to_text(events: &[SessionEvent]) -> String {
    let mut out = String::new();
    for e in events {
        match e {
            SessionEvent::UserMessage { content, .. } => {
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str("User: ");
                out.push_str(content);
            }
            SessionEvent::AssistantMessage { content, .. } => {
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str("Assistant: ");
                out.push_str(content);
            }
            _ => {}
        }
    }
    out
}

/// Translate a generic `SessionResume` into Claude's native shape.
#[allow(dead_code)]
fn claude_resume_from_generic(r: SessionResume) -> ClaudeResume {
    match r {
        SessionResume::BySessionId(id) => ClaudeResume::SessionId(id),
        SessionResume::ByTranscriptFile(path) => ClaudeResume::TranscriptFile(path),
    }
}

/// Pump `WorkerInputMessage` → `ClaudeUserMessage`. Stops when the input
/// channel closes.
fn spawn_input_translator(
    mut g: mpsc::Receiver<WorkerInputMessage>,
    c: mpsc::Sender<ClaudeUserMessage>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(msg) = g.recv().await {
            if c.send(ClaudeUserMessage {
                content: msg.content,
            })
            .await
            .is_err()
            {
                break;
            }
        }
    })
}

/// Pump `ClaudeEvent` → `WorkerEvent`. Stops when the input channel closes.
fn spawn_output_translator(
    mut c: mpsc::Receiver<ClaudeEvent>,
    g: mpsc::Sender<WorkerEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(event) = c.recv().await {
            let translated = translate_claude_event(event);
            if g.send(translated).await.is_err() {
                break;
            }
        }
    })
}

/// Per-event translation between Claude-native and generic event shapes.
/// Pulled out so unit tests can exercise it directly without spinning up the
/// translator task.
fn translate_claude_event(event: ClaudeEvent) -> WorkerEvent {
    match event {
        ClaudeEvent::Assistant { text } => WorkerEvent::AssistantText { text },
        ClaudeEvent::Usage {
            input_tokens,
            output_tokens,
            cache_read_input_tokens,
            cache_creation_input_tokens,
        } => WorkerEvent::Usage {
            usage: TokenUsage {
                input_tokens,
                output_tokens,
                cache_read_input_tokens,
                cache_creation_input_tokens,
            },
        },
        ClaudeEvent::SystemInit { session_id } => WorkerEvent::SessionInit {
            model_session_id: session_id,
        },
        ClaudeEvent::ToolUse { tool_name, payload } => WorkerEvent::ToolUse { tool_name, payload },
        ClaudeEvent::Raw { value } => WorkerEvent::Raw { value },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn decide_kind_claude_dash_prefix() {
        assert_eq!(
            ModelSelector::decide_kind(Some("claude-3-5-sonnet")),
            Kind::Claude
        );
    }

    #[test]
    fn decide_kind_opus_dash_prefix() {
        assert_eq!(
            ModelSelector::decide_kind(Some("opus-4-via-bedrock")),
            Kind::Claude
        );
    }

    #[test]
    fn decide_kind_haiku_dash_prefix() {
        assert_eq!(
            ModelSelector::decide_kind(Some("haiku-4-5-20251001")),
            Kind::Claude
        );
    }

    #[test]
    fn decide_kind_sonnet_dash_prefix() {
        assert_eq!(ModelSelector::decide_kind(Some("sonnet-4-6")), Kind::Claude);
    }

    #[test]
    fn decide_kind_bare_claude_family_names() {
        assert_eq!(ModelSelector::decide_kind(Some("claude")), Kind::Claude);
        assert_eq!(ModelSelector::decide_kind(Some("opus")), Kind::Claude);
        assert_eq!(ModelSelector::decide_kind(Some("sonnet")), Kind::Claude);
        assert_eq!(ModelSelector::decide_kind(Some("haiku")), Kind::Claude);
    }

    #[test]
    fn decide_kind_bare_claude_family_names_case_insensitive() {
        assert_eq!(ModelSelector::decide_kind(Some("Opus")), Kind::Claude);
        assert_eq!(ModelSelector::decide_kind(Some("SONNET")), Kind::Claude);
    }

    #[test]
    fn decide_kind_gpt_dash_prefix() {
        assert_eq!(ModelSelector::decide_kind(Some("gpt-4o")), Kind::Codex);
    }

    #[test]
    fn decide_kind_codex_dash_prefix() {
        assert_eq!(ModelSelector::decide_kind(Some("codex-cli")), Kind::Codex);
    }

    #[test]
    fn decide_kind_unknown_defaults_to_codex() {
        assert_eq!(ModelSelector::decide_kind(Some("unknown")), Kind::Codex);
    }

    #[test]
    fn decide_kind_none_defaults_to_codex() {
        assert_eq!(ModelSelector::decide_kind(None), Kind::Codex);
    }

    #[test]
    fn supports_interactive_true_for_claude_names() {
        assert!(ModelSelector::supports_interactive(Some(
            "claude-3-5-sonnet"
        )));
        assert!(ModelSelector::supports_interactive(Some("opus-4")));
        assert!(ModelSelector::supports_interactive(Some("sonnet-4-6")));
        assert!(ModelSelector::supports_interactive(Some(
            "haiku-4-5-20251001"
        )));
    }

    #[test]
    fn supports_interactive_true_for_bare_claude_family_names() {
        assert!(ModelSelector::supports_interactive(Some("claude")));
        assert!(ModelSelector::supports_interactive(Some("opus")));
        assert!(ModelSelector::supports_interactive(Some("sonnet")));
        assert!(ModelSelector::supports_interactive(Some("haiku")));
    }

    #[test]
    fn supports_interactive_false_for_codex_names() {
        assert!(!ModelSelector::supports_interactive(Some("gpt-4o")));
        assert!(!ModelSelector::supports_interactive(Some("codex-cli")));
        assert!(!ModelSelector::supports_interactive(Some("o1")));
        assert!(!ModelSelector::supports_interactive(Some("o3")));
        assert!(!ModelSelector::supports_interactive(Some("o4")));
    }

    #[test]
    fn supports_interactive_false_for_unknown_and_none() {
        assert!(!ModelSelector::supports_interactive(Some("unknown")));
        assert!(!ModelSelector::supports_interactive(None));
    }

    #[test]
    fn claude_resume_from_generic_by_session_id() {
        let r = claude_resume_from_generic(SessionResume::BySessionId("x".to_string()));
        match r {
            ClaudeResume::SessionId(id) => assert_eq!(id, "x"),
            other => panic!("expected SessionId, got {other:?}"),
        }
    }

    #[test]
    fn claude_resume_from_generic_by_transcript_file() {
        let p = PathBuf::from("/tmp/transcript.jsonl");
        let r = claude_resume_from_generic(SessionResume::ByTranscriptFile(p.clone()));
        match r {
            ClaudeResume::TranscriptFile(path) => assert_eq!(path, p),
            other => panic!("expected TranscriptFile, got {other:?}"),
        }
    }

    #[test]
    fn translate_assistant_event_to_assistant_text() {
        let event = ClaudeEvent::Assistant {
            text: "hi".to_string(),
        };
        let translated = translate_claude_event(event);
        match translated {
            WorkerEvent::AssistantText { text } => assert_eq!(text, "hi"),
            other => panic!("expected AssistantText, got {other:?}"),
        }
    }

    #[test]
    fn translate_usage_event_to_usage() {
        let event = ClaudeEvent::Usage {
            input_tokens: 1,
            output_tokens: 2,
            cache_read_input_tokens: 3,
            cache_creation_input_tokens: 4,
        };
        let translated = translate_claude_event(event);
        match translated {
            WorkerEvent::Usage { usage } => {
                assert_eq!(usage.input_tokens, 1);
                assert_eq!(usage.output_tokens, 2);
                assert_eq!(usage.cache_read_input_tokens, 3);
                assert_eq!(usage.cache_creation_input_tokens, 4);
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn translate_system_init_to_session_init() {
        let event = ClaudeEvent::SystemInit {
            session_id: "uuid".to_string(),
        };
        let translated = translate_claude_event(event);
        match translated {
            WorkerEvent::SessionInit { model_session_id } => assert_eq!(model_session_id, "uuid"),
            other => panic!("expected SessionInit, got {other:?}"),
        }
    }

    #[test]
    fn translate_raw_event_to_raw() {
        let value = serde_json::json!({"type": "weird", "x": 1});
        let event = ClaudeEvent::Raw {
            value: value.clone(),
        };
        let translated = translate_claude_event(event);
        match translated {
            WorkerEvent::Raw { value: v } => assert_eq!(v, value),
            other => panic!("expected Raw, got {other:?}"),
        }
    }

    #[test]
    fn translate_tool_use_event_to_tool_use() {
        let payload = serde_json::json!({"command": "ls", "description": "list"});
        let event = ClaudeEvent::ToolUse {
            tool_name: "Bash".to_string(),
            payload: payload.clone(),
        };
        let translated = translate_claude_event(event);
        match translated {
            WorkerEvent::ToolUse {
                tool_name,
                payload: p,
            } => {
                assert_eq!(tool_name, "Bash");
                assert_eq!(p, payload);
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn concat_first_message_collapses_empty_pieces() {
        assert_eq!(concat_first_message(&[], "", ""), "");
        assert_eq!(concat_first_message(&[], "prompt", ""), "prompt");
        assert_eq!(concat_first_message(&[], "", "user"), "user");
        assert_eq!(
            concat_first_message(&[], "prompt", "user"),
            "prompt\n\nuser"
        );
    }

    #[tokio::test]
    async fn spawn_output_translator_translates_event_stream() {
        let (claude_tx, claude_rx) = mpsc::channel::<ClaudeEvent>(8);
        let (worker_tx, mut worker_rx) = mpsc::channel::<WorkerEvent>(8);
        let handle = spawn_output_translator(claude_rx, worker_tx);

        claude_tx
            .send(ClaudeEvent::Assistant {
                text: "hi".to_string(),
            })
            .await
            .unwrap();
        claude_tx
            .send(ClaudeEvent::Usage {
                input_tokens: 1,
                output_tokens: 2,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
            .await
            .unwrap();
        claude_tx
            .send(ClaudeEvent::SystemInit {
                session_id: "x".to_string(),
            })
            .await
            .unwrap();
        let raw_val = serde_json::json!({"type": "weird"});
        claude_tx
            .send(ClaudeEvent::Raw {
                value: raw_val.clone(),
            })
            .await
            .unwrap();
        drop(claude_tx);
        handle.await.unwrap();

        let mut got = Vec::new();
        while let Some(e) = worker_rx.recv().await {
            got.push(e);
        }
        assert_eq!(got.len(), 4);
        assert!(matches!(got[0], WorkerEvent::AssistantText { ref text } if text == "hi"));
        assert!(matches!(
            got[1],
            WorkerEvent::Usage {
                usage: TokenUsage {
                    input_tokens: 1,
                    output_tokens: 2,
                    ..
                }
            }
        ));
        assert!(
            matches!(got[2], WorkerEvent::SessionInit { ref model_session_id } if model_session_id == "x")
        );
        assert!(matches!(got[3], WorkerEvent::Raw { ref value } if value == &raw_val));
    }

    #[tokio::test]
    async fn spawn_input_translator_passes_through_content() {
        let (worker_tx, worker_rx) = mpsc::channel::<WorkerInputMessage>(8);
        let (claude_tx, mut claude_rx) = mpsc::channel::<ClaudeUserMessage>(8);
        let handle = spawn_input_translator(worker_rx, claude_tx);

        worker_tx
            .send(WorkerInputMessage {
                content: "hello".to_string(),
            })
            .await
            .unwrap();
        drop(worker_tx);
        handle.await.unwrap();

        let msg = claude_rx.recv().await.unwrap();
        assert_eq!(msg.content, "hello");
        assert!(claude_rx.recv().await.is_none());
    }
}
