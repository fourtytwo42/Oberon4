# LLM Hotspot Diagnostics Export

**Generated:** unix:1772298580 (~day 20512 17:09 UTC)  
**Version:** v0.1.0

## Summary

| Metric | Value |
|--------|-------|
| Total requests | 4 |
| Failover events | 0 |
| Last auth rejection | none |

## Recent Errors

_No recent errors_

## Last Upstream Error

_None_

## Last Failover Chain

_No failover events recorded_

## Usage — Per Model (since last reset)

| Model | Requests | Input | Output | Cached |
|-------|----------|-------|--------|--------|
| claude-4.6-sonnet-medium | 16 | 31.0K | 306 | 0 |
| claude-4.6-sonnet-medium-thinking | 34 | 322.4K | 12.0K | 0 |
| claude-imaginary-not-real | 18 | 0 | 0 | 0 |
| claude-sonnet-4-6 | 54 | 72.2K | 753 | 0 |
| gpt-5.3-codex | 614 | 31.3M | 201.1K | 28.9M |

## Usage — Per API Key (since last reset)

### Key: default (`b8c3c605-f95b-461c-b6e7-9ed17433ae59`)

| Model | Requests | Input | Output | Cached |
|-------|----------|-------|--------|--------|
| claude-4.6-sonnet-medium | 7 | 31.0K | 306 | 0 |
| claude-4.6-sonnet-medium-thinking | 28 | 322.4K | 12.0K | 0 |
| claude-sonnet-4-6 | 22 | 72.2K | 753 | 0 |
| gpt-5.3-codex | 612 | 31.3M | 201.1K | 28.9M |

## Usage — Per OAuth Account (since last reset)

### Account: spiderman (`42b943a0-6112-4e57-b36e-870b07b7eced`)

| Model | Requests | Input | Output | Cached |
|-------|----------|-------|--------|--------|
| gpt-5.3-codex | 2 | 421.8K | 763 | 372.2K |

### Account: henderson (`4d8b11d3-6c64-49a0-87fb-b29240b36e37`)

| Model | Requests | Input | Output | Cached |
|-------|----------|-------|--------|--------|
| gpt-5.3-codex | 435 | 28.6M | 172.8K | 26.5M |

### Account: erics.cellphone (`87db8096-9948-4615-ba0c-a336cd9345fb`)

| Model | Requests | Input | Output | Cached |
|-------|----------|-------|--------|--------|
| gpt-5.3-codex | 175 | 2.3M | 27.6K | 2.0M |

### Account: spiderman (`anthropic_3dbe26a7c62a40bf86e5dd84841fc916`)

| Model | Requests | Input | Output | Cached |
|-------|----------|-------|--------|--------|
| claude-4.6-sonnet-medium | 2 | 852 | 230 | 0 |
| claude-4.6-sonnet-medium-thinking | 22 | 231.7K | 9.9K | 0 |
| claude-sonnet-4-6 | 2 | 831 | 188 | 0 |

## Recent Requests (last 10, newest first)

**unix:1772298543** `/v1/chat/completions` → `claude-4.6-sonnet-medium-thinking` | 9 msgs | stream=true
- Tools (19):** Shell, Glob, Grep, Read, Delete, StrReplace, Write, EditNotebook, TodoWrite, ReadLints, SemanticSearch, WebSearch, WebFetch, GenerateImage, AskQuestion, Task, CallMcpTool, FetchMcpResource, SwitchMode
- tool_choice: `{"type":"auto"}`
- Roles: `user -> assistant -> user -> assistant -> user -> assistant -> user -> assistant -> user`
- Last role: `user` (anthropic tool_result block on last msg: `true`)

**unix:1772298530** `/v1/chat/completions` → `claude-4.6-sonnet-medium-thinking` | 7 msgs | stream=true
- Tools (19):** Shell, Glob, Grep, Read, Delete, StrReplace, Write, EditNotebook, TodoWrite, ReadLints, SemanticSearch, WebSearch, WebFetch, GenerateImage, AskQuestion, Task, CallMcpTool, FetchMcpResource, SwitchMode
- tool_choice: `{"type":"auto"}`
- Roles: `user -> assistant -> user -> assistant -> user -> assistant -> user`
- Last role: `user` (anthropic tool_result block on last msg: `true`)

**unix:1772298516** `/v1/chat/completions` → `claude-4.6-sonnet-medium-thinking` | 5 msgs | stream=true
- Tools (19):** Shell, Glob, Grep, Read, Delete, StrReplace, Write, EditNotebook, TodoWrite, ReadLints, SemanticSearch, WebSearch, WebFetch, GenerateImage, AskQuestion, Task, CallMcpTool, FetchMcpResource, SwitchMode
- tool_choice: `{"type":"auto"}`
- Roles: `user -> assistant -> user -> assistant -> user`
- Last role: `user` (anthropic tool_result block on last msg: `true`)

**unix:1772298502** `/v1/chat/completions` → `claude-4.6-sonnet-medium-thinking` | 3 msgs | stream=true
- Tools (19):** Shell, Glob, Grep, Read, Delete, StrReplace, Write, EditNotebook, TodoWrite, ReadLints, SemanticSearch, WebSearch, WebFetch, GenerateImage, AskQuestion, Task, CallMcpTool, FetchMcpResource, SwitchMode
- tool_choice: `{"type":"auto"}`
- Roles: `user -> assistant -> user`
- Last role: `user` (anthropic tool_result block on last msg: `false`)
- Last user msg: _<open_and_recently_viewed_files>
User currently doesn't have any open files in their IDE.

Note: these files may or may ..._

