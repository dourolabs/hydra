# Metis Shell

This document describes the design for interactive shell sessions in Metis. A "shell" in this context refers to any generic interactive program—either a bash shell or an AI agent—that users can connect to and interact with in real-time.

## Motivation

The standard Metis workflow spawns agents that run to completion autonomously. However, there are scenarios where interactive sessions are valuable:

- **Debugging**: Developers may want to inspect the worker environment, run ad-hoc commands, or troubleshoot issues in real-time.
- **Guided Agent Sessions**: Users may want to interact with an AI agent conversationally, providing input and receiving responses in a loop rather than submitting a single prompt.
- **Pair Programming**: Multiple team members may want to observe or contribute to the same session simultaneously.

## Design Overview

Interactive shell sessions reuse the existing `worker_run` setup and cleanup phases but skip the `WorkerCommands` execution entirely. Instead of running a predefined AI command (Claude or Codex), the worker enters an interactive mode where stdin/stdout streams flow through metis-server to connected clients.

```
┌─────────────┐     WebSocket      ┌──────────────┐      Stream Proxy      ┌─────────────────┐
│   Client 1  │◄──────────────────►│              │◄─────────────────────►│                 │
└─────────────┘                    │              │                        │   Kubernetes    │
                                   │ metis-server │                        │      Pod        │
┌─────────────┐     WebSocket      │              │                        │  (shell proc)   │
│   Client 2  │◄──────────────────►│              │                        │                 │
└─────────────┘                    └──────────────┘                        └─────────────────┘
```

### Key Principles

1. **Streams Flow Through metis-server**: All stdin/stdout communication passes through metis-server rather than using `kubectl exec` directly. This enables access control, audit logging, and multi-client attachment without requiring direct Kubernetes access.

2. **Reuse worker_run Lifecycle**: Shell mode leverages the same setup (repository cloning, tracking branch initialization, build cache application) and cleanup (auto-commit, branch updates, patch creation) as regular task execution.

3. **Skip WorkerCommands**: The `WorkerCommands` trait execution (which runs Claude or Codex CLI) is bypassed entirely. The shell process itself becomes the interactive target.

4. **Multi-Client Attachment**: Multiple clients can attach to the same shell session. All clients receive the same stdout stream; stdin from any client is forwarded to the shell.

## Architecture

### Shell Mode Detection

The `worker_run` command detects shell mode through a task property. When shell mode is enabled:

```
worker_run (shell mode)
├── Fetch job context from metis-server
├── Clone repository (if needed)
├── Initialize tracking branches
├── Apply build cache (if configured)
├── [SKIP] WorkerCommands execution
├── Start shell process (bash or AI agent)
├── Register with metis-server for stream proxying
├── Wait for shell process to exit
├── Auto-commit uncommitted changes
├── Update tracking branches
└── Create patch artifact (if changes exist)
```

### Stream Proxying

metis-server acts as a relay between clients and the shell process running in the Kubernetes pod.

**Server Components:**

- **Shell Session Manager**: Tracks active shell sessions, their associated tasks, and connected clients.
- **Stream Proxy**: Bridges WebSocket connections from clients to the shell process streams.
- **Input Multiplexer**: Accepts stdin from multiple clients and forwards to the single shell process.
- **Output Broadcaster**: Receives stdout/stderr from the shell and broadcasts to all connected clients.

**Protocol:**

Clients connect via WebSocket to attach to a shell session. The connection supports:

- **Input frames**: Client-to-server messages containing stdin data
- **Output frames**: Server-to-client messages containing stdout/stderr data
- **Control frames**: Session metadata, resize events, disconnect notifications

### Multi-Client Attachment

Shell sessions support multiple simultaneous clients:

1. **First client creates the session**: The shell process starts when the first client attaches (or when the task is created, depending on configuration).
2. **Additional clients join**: Subsequent clients receive a replay of recent output (configurable buffer size) and then receive live output.
3. **Any client can send input**: All connected clients can send stdin; there is no exclusive input lock.
4. **Clients disconnect independently**: The shell continues running as long as at least one client is connected (or based on session timeout policy).

### API Routes

New routes for shell functionality:

```
POST   /v1/jobs/:id/shell/start     # Start shell session for a task
GET    /v1/jobs/:id/shell/attach    # WebSocket upgrade for bidirectional streaming
POST   /v1/jobs/:id/shell/resize    # Send terminal resize event
DELETE /v1/jobs/:id/shell           # Terminate shell session
GET    /v1/jobs/:id/shell/status    # Get shell session status (clients, uptime, etc.)
```

### Kubernetes Integration

Shell mode requires modifications to the Kubernetes Job specification:

- **TTY and Stdin enabled**: The container must be created with `tty: true` and `stdin: true` to support interactive input.
- **No automatic restart**: Shell jobs should not restart on exit.
- **Resource limits**: Shell sessions may have different resource requirements than batch agent tasks.

The worker pod runs a lightweight process that:
1. Connects back to metis-server to register as a shell endpoint
2. Spawns the actual shell process (bash, AI agent, etc.)
3. Proxies streams between the shell process and the metis-server connection
4. Reports shell exit status back to metis-server

## Usage Scenarios

### Interactive Bash Shell

```bash
# Create an interactive shell task
metis jobs create --repo myorg/myrepo --shell bash

# Attach to the shell
metis shell attach <task-id>

# Multiple users can attach simultaneously
# User 2 runs:
metis shell attach <task-id>
```

### Interactive AI Agent Session

```bash
# Create an interactive Claude session
metis jobs create --repo myorg/myrepo --shell claude

# Attach and interact conversationally
metis shell attach <task-id>
> What files handle authentication?
< [Claude responds]
> Please add rate limiting to the login endpoint
< [Claude makes changes]
```

### Debugging a Failed Task

```bash
# Start a shell in the same environment as a failed task
metis jobs debug <failed-task-id>

# This creates a new shell task with the same context
# allowing inspection of the environment
```

## Security Considerations

- **Authentication**: All shell connections are authenticated through the standard metis-server auth flow.
- **Authorization**: Users can only attach to shell sessions for tasks they have access to.
- **Audit Logging**: All shell input/output is logged for audit purposes.
- **Session Limits**: Configurable limits on concurrent shell sessions and session duration.
- **No Direct Kubernetes Access**: Clients never directly execute against Kubernetes; all access is mediated by metis-server.

## Future Considerations

- **Session Recording and Playback**: Store complete session transcripts for later review.
- **Collaborative Features**: Input locks, turn-taking, or designated "driver" for multi-user sessions.
- **Shell Persistence**: Allow shell sessions to survive client disconnection with background execution.
- **Custom Shell Environments**: Support for different shell types or pre-configured environments.
