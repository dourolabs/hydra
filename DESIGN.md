# Issues

Every task to be executed by the system is represented by an issue. An issue is a item for work that may span one or more tasks. 

Issues have 4 explicitly indicated statuses: Open, InProgress, Dropped, Closed. 
Issues also have 2 inferred states: Ready, NotReady, that indicate whether or not the issue is ready to be worked on.
Open issues are Ready unless they have a blocked-on edge to an issue that isn't Closed (dropped issues do not unblock their downstream dependencies)
InProgress issues are Ready if all of their children are Closed
Dropped issues are never Ready; they remain blocking for downstream work until users intervene.
Whenever an issue is Ready, an agent may be spawned to work on it. When an issue is marked Dropped via the metis-server API, any tasks spawned from it are terminated immediately.

The workflow to process an issue is as follows
- the Issue is created. 
- once the issue is Ready, an agent (or human) is assigned to work on it
- The agent works on it.
  - if the agent finishes it, it sets the status to Closed
  - if the agent does not finish it, it sets the status to InProgress and creates new child issues to record future work
- At this point, the issue should be InProgress. Once the issue is Ready again, another agent will be assigned to work on it
  - if the agent determines the issue is complete (most likely), then it sets the status to Closed
  - otherwise, it can repeat the process of identifying more work and scheduling it. 

An issue cannot be marked as Closed unless all of its child issues are Closed.

## Example: a simple PR task

1. Human creates an issue A 
2. The Agent who addresses this (1) makes a patch and submits it, then (2) creates an issue B requesting it to be merged. B --child-of--> A 
3. The Agent who works on the merge request issue reviews the patch, if the patch is good, the agent leaves a review, merges it, then closes B.
4. The Agent working on the merge request can request reviews for the patch by making a ticket C --child-of--> B and assigning it to someone.


# Workers & Git

The system creates tracking branches that are pushed up to the remote to track the work
done by agents for both issues and tasks. There are 4 branches created by any given task:
* `metis/<issue-id>/base` tracks where the work for an issue started
* `metis/<issue-id>/head` tracks the current head of the work for the issue. 
* `metis/<task-id>/base` tracks where the work for a task started
* `metis/<task-id>/head` tracks where the work for a task ended.

Workers working on a specific issue try to start from `metis/<issue-id>/head`, which allows
them to pick up the work from previous workers on the same issue. This approach is similar to running the agent in a loop on a single machine (though any changes not tracked by git are lost between agent runs).

The branch invariants above are maintained by the `worker_run` command. It creates these tracking branches on startup, and then whenever the worker ends, `worker_run` will commit any uncommitted changes, push them up, and update the branch refs. 

Another neat thing about this 