# Task Brief: Zip Plugin Archive Install

Objective:
- implement a mod-style plugin/package install path using zip archives and generated config files

Must:
- support `.obplg.zip` and `.obpkg.zip`
- generate `~/.oberon4/plugin-config/<id>.toml`
- preserve operator-edited settings
- keep secrets out of generated config
- fail closed on invalid archive or invalid config

Must not:
- expose raw host paths or secrets to agents
- introduce a remote installer
- break the canonical host tool workflow
