# Metis

Metis is an agent coordination framework for running simultaneous Agentic AI coding agents. 
The system is designed to give developers *maximum leverage* -- it maximizes development velocity by leaning on Agentic AI as much as possible.

## Motivation

Agentic AI coding tools such as Claude and Codex are very powerful tools for writing software.
The speed at which they can generate code is much, much faster than a human developer.
However, current ways of working with these agents have several limitations:
* It's difficult to juggle multiple agents working on different tasks. Most of the time an agent is working,
  the developer is idle. You only need to intervene when the agent has something to review, or a question.
* The quality of the generated code can be questionable. Sometimes it's fine, and sometimes it's total garbage. 
  You can't let the agents run wild on your codebase without causing problems.

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
`Open` issues are `Ready` unless they have a blocked-on edge to an issue that isn't `Closed` (`Dropped` issues do not unblock their downstream dependencies)
`InProgress` issues are `Ready` if all of their children are `Closed`
`Dropped` issues are never `Ready`; they remain blocking for downstream work until users intervene.
Whenever an issue is `Ready`, an agent may be spawned to work on it.
When an issue is marked `Dropped` via the metis-server API, any tasks spawned from it are terminated immediately.

Agents will be spawned for any `Ready` issues that are assigned to an AI agent. 
The agent works on the task and updates the issue tracker (via the `metis` CLI) as it goes.
When the agent starts, it sets the status to `InProgress`. When it finishes, it sets the status to `Closed`.
The agent may also end its session while the issue is `InProgress` -- this can happen for multiple reasons,
including that the agent is waiting for an async action from another party (such as a code review), or the agent 
decided it couldn't finish the task in one session. If this happens, another agent may be spawned to work on
the task (either immediately or once the async action completes). The state of the git repository is preserved
between sequential tasks running on the same issue, which enables this follow-up agent to work off of the 
results produced by the first agent. See details in the section below.

The system implements several guards An issue cannot be marked as Closed unless all of its child issues are Closed.

## Example: a simple PR task

1. Human creates an issue A 
2. The Agent who addresses this (1) makes a patch and submits it, then (2) creates an issue B requesting it to be merged. B --child-of--> A 
3. The Agent who works on the merge request issue reviews the patch, if the patch is good, the agent leaves a review, merges it, then closes B.
4. The Agent working on the merge request can request reviews for the patch by making a ticket C --child-of--> B and assigning it to someone.


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


### The Merge Queue