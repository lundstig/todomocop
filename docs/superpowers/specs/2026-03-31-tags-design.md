# Tags Design

Lightweight tagging system for grouping tasks across overlapping "projects" or batches.

## Motivation

Tasks often share a common theme (e.g. multiple audit findings, search quality issues) but should remain separate tickets. Workspaces serve as a trust/data boundary and are not suitable for this — tags provide an orthogonal, overlapping organizational dimension.

Core use case: after processing a batch of related tasks, label them all with a tag like `audit` so they can be listed and operated on as a group later.

## Data Model (Schema v3)

### `tags` table

| Column        | Type   | Constraints     | Notes                          |
|---------------|--------|-----------------|--------------------------------|
| (internal id) | i64    | auto-increment  | rust-query managed, not exposed |
| `name`        | String | unique, required | Display name, renamable         |
| `description` | String | default ""       | User-editable context           |

### `task_tags` join table

| Column | Type | Constraints           | Notes                    |
|--------|------|-----------------------|--------------------------|
| `task`  | FK   | references tasks      | Foreign key to tasks table |
| `tag`   | FK   | references tags       | Foreign key to tags table  |

Unique constraint on (task, tag) — a task cannot have the same tag twice.

### Relationship to workspaces

Tags are global entities, not workspace-scoped. A single `audit` tag can appear on tasks in both "work" and "personal" workspaces.

**Sync boundary:** When syncing a workspace, only tags that have at least one task in that workspace are synced. Tag entities with no tasks in the target workspace are invisible to that sync. This preserves the workspace data boundary — tag names from another workspace are never leaked.

## Naming Convention

Tags are free-form strings. A `prefix:value` convention is encouraged but not enforced:

- `repo:tandemhealth`, `repo:todomocop` — repository identification
- `batch:audit-q1`, `project:search-quality` — grouping
- `audit`, `urgent` — plain strings are also fine

This convention is documented in MCP tool descriptions to nudge the agent toward consistent usage.

## Tag Lifecycle

- **Auto-creation:** Tagging a task with a name that doesn't exist creates the tag automatically with an empty description. No upfront ceremony required.
- **Rename:** Updates `tags.name`. All task associations follow via the internal key — no task rows touched.
- **Edit description:** Update `tags.description` at any time.
- **Delete:** Removes the tag entity and all task-tag associations. Does not affect the tasks themselves.

## MCP Tools

### New tools

| Tool         | Parameters                                          | Behavior                                                    |
|--------------|-----------------------------------------------------|-------------------------------------------------------------|
| `tag_task`   | `task_id`, `tag` (name string)                      | Add tag to task. Auto-creates tag if new.                   |
| `untag_task` | `task_id`, `tag` (name string)                      | Remove tag from task.                                       |
| `list_tags`  | `workspace` (optional)                              | List all tags. If workspace given, only tags with tasks in that workspace. Shows name, description, task count. |
| `edit_tag`   | `tag` (current name), `name` (optional), `description` (optional) | Rename and/or set description.                              |
| `delete_tag` | `tag` (name string)                                 | Delete tag and all associations.                            |

### Changes to existing tools

- `add_task`: New optional `tags` parameter (list of strings) for tagging at creation time.
- `list_tasks` and `search`: New optional `tag` filter parameter.
- Task output includes tags:
  - **Compact:** Tags appended inline, e.g. `#1 [ready] Fix search ranking (work) p1 due:2026-04-01 [audit, repo:tandemhealth]`
  - **Verbose:** Full tag objects (name + description) in JSON output.

### MCP tool description guidance

The `tag_task` tool description includes:

> "Tags are free-form strings. Convention: use `prefix:value` for categorization (e.g. `repo:tandemhealth`, `batch:audit-q1`, `project:search-quality`). Plain strings are also fine (e.g. `audit`, `urgent`). Tags are auto-created on first use — no need to create them beforehand."

### `list_tags` output

Includes task count per tag so the agent can see at a glance what's active. When `workspace` is provided, the task count reflects only tasks in that workspace.

## CLI

### New commands

- `todomocop tags` — list all tags (with task counts)
- `todomocop tag-edit <tag> --name <new-name> --description <desc>` — rename and/or set description
- `todomocop tag-delete <tag>` — delete tag and associations

### Changes to existing commands

- `todomocop add --tag <tag> [--tag <tag> ...]` — tag at creation time
- `todomocop tag <task_id> <tag>` — add tag to existing task
- `todomocop untag <task_id> <tag>` — remove tag from task
- `todomocop tasks --tag <tag>` — filter task list by tag

## Testing

- Unit tests in `core/src/tag.rs` (new file) using `Db::open_in_memory()`
- Cover: auto-creation, rename, delete cascading, duplicate prevention, workspace-filtered listing, tag filter on list_tasks
