# Metis

Metis is an agent coordination framework for running multiple simultaneous Agentic AI coding agents. 
The system is designed to give developers *maximum leverage* -- it maximizes development velocity by leaning on Agentic AI as much as possible.

Metis makes you the manager of an engineering team. Instead of operating on the level of code, you operate on the level of 
tasks -- what needs to be done? You work with an issue tracker, and you use it to assign work to your team.
Agents on your team take care of the implementation details. You survey their progress, review their work, and 
offer course corrections as needed. You use the issue tracker is a memory aid to make sure your team does
all the work that's needed.

## Motivation

Agentic AI coding tools such as Claude and Codex are very powerful tools for writing software.
The speed at which they can generate code is much faster than a human developer.
However, the current interactive paradigm for working with these agents is very limiting.
Most of the time when an agent is working, the developer is idle. The agents also ask questions or 
for approvals in the middle of the task, which means the developer can't walk away.

Developers try to solve these problems by running multiple agents at the same time, for example, in
multiple terminal windows. However, it's difficult to manage more than a few agents this way, as you
keep context-switching between terminals.

## Design

Metis coordinates humans and agents using an issue tracker. Issues represent work to be done, and are assigned to
someone to perform it, who could be either a human or an agent. Issues have a graph structure that allows the
system to determine what can currently be worked on. The system then spawns AI agents to work on any tasks that are ready.

A fundamental design choice is that agents and humans are equivalent in the system -- they both interact with
the system using the same tool, the `metis` CLI. This choice means that any work that humans can perform can also be 
delegated to agents. For example, a complex issue may need to be broken down into smaller subtasks to ensure success --
an AI agent can do that. Or a pull request may need to be reviewed by someone before it can be merged -- an AI agent can
do that too.

### Issues

All work to be performed by the system is represented by issues.
An issue is essentially the same as an issue in any task tracking system you are familiar with -- it's a text description of what needs to be done, with some added metadata fields.
Issues have 4 explicitly indicated statuses: `Open`, `InProgress`, `Dropped`, `Closed`. 
Issues additionally have a graph structure with two types of relationships `x:blocked-on:y` and `x:child-of:y`.

The system uses the combination of the status and graph structure to determine what issues can be worked on.
Issues also have 2 inferred states: `Ready`, `NotReady`, that indicate whether or not the issue is ready to be worked on.
`Open` issues are `Ready` unless they have a blocked-on edge to an issue that isn't `Closed`, or they are a child of a parent in a terminal failure state (`Dropped`, `Rejected`, or `Failed`). Children of `Closed` parents remain ready since the parent's work completed successfully.
`InProgress` issues are `Ready` if all of their children are in a terminal state (`Closed`, `Dropped`, `Rejected`, or `Failed`).
`Dropped` issues are never `Ready`; they remain blocking for downstream work until users intervene.
Whenever an issue is `Ready`, an agent may be spawned to work on it.
When an issue is marked `Dropped` via the metis-server API, its children are recursively set to `Dropped` (since the work is explicitly cancelled), and any tasks spawned from it are terminated immediately.
`Rejected` and `Failed` issues do not cascade status changes to their children or blocked-on dependents; instead, blocking is retained (the dependent issues remain in their current status but are not ready to run).

Agents will be spawned for any `Ready` issues that are assigned to an AI agent. 
The agent works on the task and updates the issue tracker (via the `metis` CLI) as it goes.
When the agent starts, it sets the status to `InProgress`. When it finishes, it sets the status to `Closed`.
The agent may also end its session while the issue is `InProgress` -- this can happen for multiple reasons,
including that the agent is waiting for an async action from another party (such as a code review), or the agent 
decided it couldn't finish the task in one session. If this happens, another agent may be spawned to work on
the task (either immediately or once the async action completes). The state of the git repository is preserved
between sequential tasks running on the same issue, which enables this follow-up agent to work off of the 
results produced by the first agent. See details in the section below.

**Example: a simple PR task**

1. User creates issue `A`
2. An agent is spawned to work on `A`. It makes a patch and submits it, then creates issue `B` requesting a review of the patch. `B:child-of:A`
3. User reviews the patch (either accepting or rejecting it). Submitting a review automatically closes the issue `B`, as the review was completed.
3. Issue `A` is now ready to be worked on again. An agent is spawned to work on it. The agent reviews the history of what has happened so far and
   determines if the issue has been completed. If it has (e.g., the patch was accepted), then the agent marks the issue as `Closed`. Otherwise,
   the agent goes back to step 2 and tries to make another patch.

### Git State Management

The system creates tracking branches that are pushed up to the remote to track the work
done by agents for both issues and tasks. There are 4 branches created by any given task:
* `metis/<issue-id>/base` tracks where the work for an issue started
* `metis/<issue-id>/head` tracks the current head of the work for the issue. 
* `metis/<task-id>/base` tracks where the work for a task started
* `metis/<task-id>/head` tracks where the work for a task ended.

Workers working on a specific issue try to start from `metis/<issue-id>/head`, which allows
them to pick up the work from previous workers on the same issue. This approach is similar to running the agent in a loop on a single machine (though any changes not tracked by git are lost between agent runs).

The branch invariants above are maintained by the `worker_run` command. It creates these tracking branches on startup, and then whenever the worker ends, `worker_run` will commit any uncommitted changes, push them up, and update the branch refs. 

Note that all of these branches are pushed to the remote, so you can easily fetch the repo
state before/after the work of a task / issue. Simply checkout the corresponding branch from
the remote.
