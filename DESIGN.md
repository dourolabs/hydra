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

## Example: Task with rejected PR
1. 
