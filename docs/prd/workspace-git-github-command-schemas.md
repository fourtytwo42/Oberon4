# Oberon4 Workspace, Git, and GitHub Command Schemas

Version: 0.1
Owner: Oberon4
Status: draft

Purpose:
- define concrete initial command schemas for the first versioned workspace tool package

Companion documents:
- `docs/prd/PRD-versioned-workspace-git.md`
- `docs/prd/tool-interaction-contract.md`

## 1. Workspace Commands

### `workspace.read_file`
Input:
```json
{
  "workspaceHandle": "wrk_123",
  "path": "docs/notes.md"
}
```

Output:
```json
{
  "workspaceHandle": "wrk_123",
  "workspaceRevision": "wsr_123",
  "path": "docs/notes.md",
  "content": "hello",
  "truncated": false
}
```

### `workspace.list_dir`
Input:
```json
{
  "workspaceHandle": "wrk_123",
  "path": "docs"
}
```

Output:
```json
{
  "workspaceHandle": "wrk_123",
  "workspaceRevision": "wsr_123",
  "path": "docs",
  "entries": [
    {
      "name": "notes.md",
      "kind": "file"
    }
  ]
}
```

### `workspace.stat_path`
Input:
```json
{
  "workspaceHandle": "wrk_123",
  "path": "docs/notes.md"
}
```

Output:
```json
{
  "workspaceHandle": "wrk_123",
  "workspaceRevision": "wsr_123",
  "path": "docs/notes.md",
  "kind": "file",
  "size": 128
}
```

### `workspace.write_file_atomic`
Input:
```json
{
  "workspaceHandle": "wrk_123",
  "path": "docs/notes.md",
  "content": "hello",
  "expectedRevision": "wsr_123"
}
```

Output:
```json
{
  "workspaceHandle": "wrk_123",
  "workspaceRevision": "wsr_124",
  "logicalPath": "docs/notes.md",
  "operation": "write_file_atomic",
  "bytesWritten": 5,
  "preconditionStatus": "matched",
  "touched": true
}
```

### `workspace.apply_patch`
Input:
```json
{
  "workspaceHandle": "wrk_123",
  "patch": "*** Begin Patch\n...",
  "expectedRevision": "wsr_123"
}
```

Output:
```json
{
  "workspaceHandle": "wrk_123",
  "workspaceRevision": "wsr_124",
  "operation": "apply_patch",
  "preconditionStatus": "matched",
  "touchedPaths": ["docs/notes.md"]
}
```

## 2. Git Commands

### `git.status`
Input:
```json
{
  "workspaceHandle": "wrk_123"
}
```

Output:
```json
{
  "workspaceHandle": "wrk_123",
  "workspaceRevision": "wsr_124",
  "branch": "feature/docs",
  "dirty": true,
  "filesChanged": 1
}
```

### `git.diff`
Input:
```json
{
  "workspaceHandle": "wrk_123"
}
```

Output:
```json
{
  "workspaceHandle": "wrk_123",
  "workspaceRevision": "wsr_124",
  "branch": "feature/docs",
  "filesChanged": 1,
  "diffArtifact": "art_123"
}
```

### `git.branch.create`
Input:
```json
{
  "workspaceHandle": "wrk_123",
  "name": "feature/docs"
}
```

Output:
```json
{
  "workspaceHandle": "wrk_123",
  "branch": "feature/docs",
  "created": true
}
```

### `git.commit`
Input:
```json
{
  "workspaceHandle": "wrk_123",
  "message": "Update notes"
}
```

Output:
```json
{
  "workspaceHandle": "wrk_123",
  "branch": "feature/docs",
  "headCommit": "abc123",
  "dirty": false,
  "filesChanged": 1
}
```

### `git.push`
Input:
```json
{
  "workspaceHandle": "wrk_123",
  "remote": "origin",
  "branch": "feature/docs"
}
```

Output:
```json
{
  "remote": "origin",
  "branch": "feature/docs",
  "commit": "abc123",
  "pushed": true,
  "provider": "github",
  "remoteRef": "refs/heads/feature/docs"
}
```

## 3. GitHub Commands

### `github.auth_status`
Input:
```json
{}
```

Output:
```json
{
  "provider": "github",
  "authenticated": true,
  "securityLevel": "os-protected"
}
```

### `github.clone_to_workspace`
Input:
```json
{
  "repo": "owner/repo"
}
```

Output:
```json
{
  "provider": "github",
  "workspaceHandle": "wrk_123",
  "defaultBranch": "main"
}
```

### `github.repo_info`
Input:
```json
{
  "repo": "owner/repo"
}
```

Output:
```json
{
  "provider": "github",
  "repo": "owner/repo",
  "defaultBranch": "main",
  "private": true
}
```

## 4. Acceptance Rules
- `WGS-101`: initial workspace command contracts are concrete and conflict-aware
- `WGS-102`: git command contracts are structured and shell-independent
- `WGS-103`: GitHub command contracts rely on brokered auth rather than raw token handling
